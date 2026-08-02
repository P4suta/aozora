#![expect(
    clippy::expect_used,
    reason = "projection state maintains a non-empty frame stack and infallible buffers"
)]

//! Source-driven projection from a [`Snapshot`] to a
//! [`pandoc_ast::Pandoc`] document.
//!
//! Walks the source linearly, slicing it into spans by the owned
//! `source_nodes` side-table. Plain runs flow into Pandoc inlines
//! verbatim (with `\n\n` paragraph splits and single `\n` →
//! `SoftBreak`). Each classified node lifts to a Pandoc inline /
//! block construct as documented in [`crate`]. Payload text is resolved
//! through the output's [`NodeStore`].

use core::mem;

use pandoc_ast::{Attr, Block, Inline, Pandoc};

use crate::Snapshot;
use crate::pandoc::AOZORA_CLASS_PREFIX;
use crate::pipeline::lex;
use crate::render::MAX_NESTED_SOURCE_DEPTH;
use crate::spec::roman_slug;
use crate::syntax::accent::{compose_accent, compose_accent_dots};
use crate::syntax::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardFormat, ForwardPayload, Gaiji, Heading,
    HeadingHint, Illustration, Kaeriten, MarginNote, Node, NodeRef, NodeStore, Ruby, Segment,
    SourceNode,
};
use crate::syntax::format::Format;
use crate::syntax::{
    AbsoluteSize, AccentMark, BoutenKind, BoutenPosition, DirectiveKind, EnclosureKind, FontShift,
    ForwardAttr, ForwardOrigin, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, LineFormat,
    MarginNoteKind, RegionFormat, RubySide, SectionKind,
};

/// Lift a parsed [`Snapshot`] to a [`pandoc_ast::Pandoc`] document.
///
/// See the crate-level docs for the projection rules.
#[must_use]
pub fn to_pandoc(snapshot: &Snapshot) -> Pandoc {
    let out = snapshot.output();
    // `source_nodes` index into the sanitize-stage buffer, not the raw
    // user-supplied source. The owned lex output carries exactly that buffer
    // in `sanitized`, so the slice base already matches the source-node
    // coordinate system — no re-sanitize is needed.
    let mut converter = Converter::new(&out.sanitized, &out.source_nodes, &out.store, 0);
    converter.run();
    Pandoc {
        meta: pandoc_ast::Map::new(),
        blocks: converter.blocks,
        // The pandoc 3.x JSON API version. `pandoc_ast` accepts 1.20 or
        // newer, so an older reader still parses this output.
        pandoc_api_version: vec![1, 23],
    }
}

// ---------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------

struct BlockFrame {
    blocks: Vec<Block>,
    inlines: Option<Vec<Inline>>,
    container: Option<RegionFormat>,
}

impl BlockFrame {
    fn root() -> Self {
        Self {
            blocks: Vec::new(),
            inlines: None,
            container: None,
        }
    }

    fn container(kind: RegionFormat) -> Self {
        Self {
            blocks: Vec::new(),
            inlines: None,
            container: Some(kind),
        }
    }

    fn paragraph(&mut self) -> &mut Vec<Inline> {
        self.inlines.get_or_insert_with(Vec::new)
    }

    fn flush_paragraph(&mut self) {
        if let Some(mut inlines) = self.inlines.take()
            && {
                while matches!(inlines.last(), Some(Inline::SoftBreak)) {
                    inlines.pop();
                }
                !inlines.is_empty()
            }
        {
            self.blocks.push(Block::Para(inlines));
        }
    }
}

struct InlineFrame {
    container: InlineContainer,
    inlines: Vec<Inline>,
    emitted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineContainer {
    Region(RegionFormat),
    Warichu,
}

enum Frame {
    Block(BlockFrame),
    Inline(InlineFrame),
}

struct Converter<'src> {
    source: &'src str,
    nodes: &'src [SourceNode],
    /// Backing store the owned nodes' `StrId` / range payloads resolve against.
    store: &'src NodeStore,
    stack: Vec<Frame>,
    /// Cursor into `source` (byte offset).
    cursor: usize,
    pending_newlines: usize,
    nested_depth: usize,
    inline_source: bool,
    /// Final document blocks, populated by [`Converter::run`] from
    /// the root frame on completion.
    blocks: Vec<Block>,
}

impl<'src> Converter<'src> {
    fn new(
        source: &'src str,
        nodes: &'src [SourceNode],
        store: &'src NodeStore,
        nested_depth: usize,
    ) -> Self {
        Self {
            source,
            nodes,
            store,
            stack: vec![Frame::Block(BlockFrame::root())],
            cursor: 0,
            pending_newlines: 0,
            nested_depth,
            inline_source: false,
            blocks: Vec::new(),
        }
    }

    fn inline_source(
        source: &'src str,
        nodes: &'src [SourceNode],
        store: &'src NodeStore,
        nested_depth: usize,
    ) -> Self {
        let mut converter = Self::new(source, nodes, store, nested_depth);
        converter.inline_source = true;
        converter
    }

    fn run(&mut self) {
        for entry in self.nodes {
            // Plain run between previous cursor and this node.
            self.flush_plain(entry.source_span.start as usize);
            self.resolve_pending_newlines();
            self.dispatch_node(entry);
            self.cursor = entry.source_span.end as usize;
        }
        self.flush_plain(self.source.len());
        while self.stack.len() > 1 {
            let frame = self.stack.pop().expect("non-empty stack");
            self.close_frame(frame);
        }
        self.flush_paragraph();
        let Frame::Block(mut root) = self.stack.pop().expect("root frame") else {
            unreachable!("the bottom frame is always the document root")
        };
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
        if self.inline_source {
            let mut text_start = 0;
            for (offset, _) in chunk.match_indices('\n') {
                if text_start < offset {
                    self.push_inline(Inline::Str(chunk[text_start..offset].to_owned()));
                }
                if self.cursor.saturating_add(offset).saturating_add(1) < self.source.len() {
                    self.push_inline(Inline::LineBreak);
                }
                text_start = offset.saturating_add(1);
            }
            if text_start < chunk.len() {
                self.push_inline(Inline::Str(chunk[text_start..].to_owned()));
            }
            self.cursor = end;
            return;
        }
        let mut text_start = 0;
        for (offset, _) in chunk.match_indices('\n') {
            if text_start < offset {
                self.resolve_pending_newlines();
                self.push_inline(Inline::Str(chunk[text_start..offset].to_owned()));
            }
            self.pending_newlines += 1;
            text_start = offset + 1;
        }
        if text_start < chunk.len() {
            self.resolve_pending_newlines();
            self.push_inline(Inline::Str(chunk[text_start..].to_owned()));
        }
        self.cursor = end;
    }

    fn resolve_pending_newlines(&mut self) {
        match mem::take(&mut self.pending_newlines) {
            0 => {}
            1 if self.current_inline_is_empty() => {}
            1 => self.push_inline(Inline::SoftBreak),
            _ => self.flush_paragraph(),
        }
    }

    fn current_inline_is_empty(&self) -> bool {
        let block_index = self.current_block_index();
        self.stack[block_index..].iter().all(|frame| match frame {
            Frame::Block(frame) => frame.inlines.as_ref().is_none_or(Vec::is_empty),
            Frame::Inline(frame) => frame.inlines.is_empty(),
        })
    }

    fn push_inline(&mut self, inline: Inline) {
        let index = self.stack.len().checked_sub(1).expect("root frame");
        self.push_inline_at(index, inline);
    }

    fn push_inline_at(&mut self, index: usize, inline: Inline) {
        match &mut self.stack[index] {
            Frame::Block(frame) => frame.paragraph().push(inline),
            Frame::Inline(frame) => frame.inlines.push(inline),
        }
    }

    fn current_block_index(&self) -> usize {
        self.stack
            .iter()
            .rposition(|frame| matches!(frame, Frame::Block(_)))
            .expect("root frame")
    }

    fn materialize_inline_fragments(&mut self) {
        let block_index = self.current_block_index();
        for index in (block_index + 1..self.stack.len()).rev() {
            let Frame::Inline(frame) = &mut self.stack[index] else {
                unreachable!("the nearest block bounds a run of inline frames")
            };
            let inlines = mem::take(&mut frame.inlines);
            if inlines.is_empty() && frame.emitted {
                continue;
            }
            frame.emitted = true;
            let inline = inline_container(frame.container, inlines);
            self.push_inline_at(index - 1, inline);
        }
    }

    fn flush_paragraph(&mut self) {
        self.materialize_inline_fragments();
        let index = self.current_block_index();
        let Frame::Block(frame) = &mut self.stack[index] else {
            unreachable!("current_block_index returns a block frame")
        };
        frame.flush_paragraph();
    }

    fn push_block(&mut self, block: Block) {
        let index = self.current_block_index();
        let Frame::Block(frame) = &mut self.stack[index] else {
            unreachable!("current_block_index returns a block frame")
        };
        frame.blocks.push(block);
    }

    fn dispatch_node(&mut self, entry: &SourceNode) {
        match entry.node {
            NodeRef::Inline(node) | NodeRef::BlockLeaf(node) => self.dispatch_leaf(node),
            NodeRef::BlockOpen(kind) => self.open_container(kind),
            NodeRef::BlockClose(close) => self.close_container(close.is_inline()),
        }
    }

    fn dispatch_leaf(&mut self, node: Node) {
        use Node as N;
        if let N::Directive(directive) = node {
            match directive.kind {
                DirectiveKind::WarichuOpen => {
                    self.open_warichu();
                    return;
                }
                DirectiveKind::WarichuClose if self.close_warichu() => return,
                _ => {}
            }
        }
        let store = self.store;
        let block = match node {
            N::PageBreak => Some(Block::HorizontalRule),
            N::BodyEnd => Some(Block::Div(
                (
                    String::new(),
                    vec![format!("{AOZORA_CLASS_PREFIX}body-end")],
                    Vec::new(),
                ),
                Vec::new(),
            )),
            N::SectionBreak(kind) => Some(section_break_block(kind)),
            N::Heading(heading) => Some(aozora_heading_block(heading, store, self.nested_depth)),
            N::Illustration(illustration) => {
                Some(sashie_block(illustration, store, self.nested_depth))
            }
            N::Ruby(_)
            | N::Format(_)
            | N::Gaiji(_)
            | N::Line(_)
            | N::ForcedBreak
            | N::HeadingHint(_)
            | N::Kaeriten(_)
            | N::Directive(_)
            | N::AngleQuote(_)
            | N::MarginNote(_) => None,
        };
        if let Some(block) = block {
            self.flush_paragraph();
            self.push_block(block);
        } else if let Some(inline) = node_inline(node, store, self.nested_depth) {
            self.push_inline(inline);
        }
    }

    fn open_container(&mut self, kind: RegionFormat) {
        if kind.is_inline() {
            self.stack.push(Frame::Inline(InlineFrame {
                container: InlineContainer::Region(kind),
                inlines: Vec::new(),
                emitted: false,
            }));
        } else {
            self.flush_paragraph();
            self.stack.push(Frame::Block(BlockFrame::container(kind)));
        }
    }

    fn open_warichu(&mut self) {
        self.stack.push(Frame::Inline(InlineFrame {
            container: InlineContainer::Warichu,
            inlines: Vec::new(),
            emitted: false,
        }));
    }

    fn close_warichu(&mut self) -> bool {
        let Some(index) = self.stack.iter().rposition(|frame| {
            matches!(
                frame,
                Frame::Inline(InlineFrame {
                    container: InlineContainer::Warichu,
                    ..
                })
            )
        }) else {
            return false;
        };
        self.close_frame_at(index)
    }

    fn close_container(&mut self, closing_inline: bool) {
        if self.stack.len() <= 1 {
            return;
        }
        if closing_inline {
            if let Some(index) = self.stack.iter().rposition(|frame| {
                matches!(
                    frame,
                    Frame::Inline(InlineFrame {
                        container: InlineContainer::Region(_),
                        ..
                    })
                )
            }) {
                self.close_frame_at(index);
            }
            return;
        }
        if let Some(index) = self.stack.iter().rposition(|frame| {
            matches!(
                frame,
                Frame::Block(BlockFrame {
                    container: Some(_),
                    ..
                })
            )
        }) {
            self.close_frame_at(index);
        }
    }

    fn close_frame_at(&mut self, index: usize) -> bool {
        if self.stack[index + 1..]
            .iter()
            .any(|frame| matches!(frame, Frame::Block(_)))
        {
            return false;
        }
        let mut reopen = Vec::new();
        while self.stack.len() > index.saturating_add(1) {
            let frame = self.stack.pop().expect("inline frame above target");
            let Frame::Inline(inline) = frame else {
                unreachable!("block frames above the target were rejected")
            };
            reopen.push(inline.container);
            self.close_frame(Frame::Inline(inline));
        }
        let frame = self.stack.pop().expect("target inline frame");
        self.close_frame(frame);
        for container in reopen.into_iter().rev() {
            self.stack.push(Frame::Inline(InlineFrame {
                container,
                inlines: Vec::new(),
                emitted: false,
            }));
        }
        true
    }

    fn close_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Inline(frame) => {
                if !frame.inlines.is_empty() || !frame.emitted {
                    self.push_inline(inline_container(frame.container, frame.inlines));
                }
            }
            Frame::Block(mut frame) => {
                frame.flush_paragraph();
                let kind = frame
                    .container
                    .expect("only the root block frame has no container");
                self.push_block(region_block(kind, frame.blocks));
            }
        }
    }
}

// ---------------------------------------------------------------------
// Per-variant inline / block builders
// ---------------------------------------------------------------------

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

/// Resolve a [`ContentRange`] payload field (ruby base/reading, forward
/// target, …) to its Pandoc inlines.
fn content_range_to_inlines(
    range: ContentRange,
    store: &NodeStore,
    nested_depth: usize,
) -> Vec<Inline> {
    let mut buf = Vec::new();
    for &content in store.resolve_content_range(range) {
        push_content_inlines(content, store, nested_depth, &mut buf);
    }
    buf
}

/// Resolve a bare [`Content`] payload field (warichu upper/lower,
/// sashie caption) to its Pandoc inlines.
fn content_to_inlines(content: Content, store: &NodeStore, nested_depth: usize) -> Vec<Inline> {
    let mut buf = Vec::new();
    push_content_inlines(content, store, nested_depth, &mut buf);
    buf
}

fn nested_content_to_inlines(
    content: Content,
    store: &NodeStore,
    nested_depth: usize,
) -> Vec<Inline> {
    if nested_depth < MAX_NESTED_SOURCE_DEPTH
        && let Content::Plain(id) = content
        && let Some(inlines) =
            nested_source_to_inlines(store.resolve_str(id), nested_depth.saturating_add(1))
    {
        return inlines;
    }
    content_to_inlines(content, store, nested_depth)
}

fn push_content_inlines(
    content: Content,
    store: &NodeStore,
    nested_depth: usize,
    buf: &mut Vec<Inline>,
) {
    match content {
        Content::Plain(id) => buf.push(Inline::Str(store.resolve_str(id).to_owned())),
        Content::Segments(range) => {
            for &seg in store.resolve_seg_range(range) {
                match seg {
                    Segment::Text(id) => {
                        buf.push(Inline::Str(store.resolve_str(id).to_owned()));
                    }
                    Segment::Gaiji(g) => buf.push(gaiji_inline(g, store)),
                    Segment::Directive(a) => buf.push(annotation_inline(a, store)),
                    Segment::Node(node) => {
                        if let Some(inline) = nested_node_inline(node, store, nested_depth) {
                            buf.push(inline);
                        }
                    }
                }
            }
        }
    }
}

fn nested_node_inline(node: Node, store: &NodeStore, nested_depth: usize) -> Option<Inline> {
    match node {
        Node::Illustration(illustration) => Some(sashie_inline(illustration, store, nested_depth)),
        _ => node_inline(node, store, nested_depth),
    }
}

fn node_inline(node: Node, store: &NodeStore, nested_depth: usize) -> Option<Inline> {
    use Node as N;
    if let N::Format(f) = node
        && matches!(f.origin, ForwardOrigin::Referenced)
    {
        return None;
    }
    Some(match node {
        N::Ruby(r) => ruby_inline(r, store, nested_depth),
        N::MarginNote(s) => side_note_inline(s, store, nested_depth),
        N::Format(f) => format_inline(f, store, nested_depth),
        N::Gaiji(g) => gaiji_inline(g, store),
        N::Line(lf) => line_inline(lf),
        N::Directive(a) => annotation_inline(a, store),
        N::Kaeriten(k) => kaeriten_inline(k, store),
        N::AngleQuote(d) => angle_quote_inline(d, store, nested_depth),
        N::HeadingHint(h) => heading_hint_inline(h, store),
        N::ForcedBreak => Inline::LineBreak,
        N::PageBreak | N::SectionBreak(_) | N::BodyEnd | N::Heading(_) | N::Illustration(_) => {
            return None;
        }
    })
}

fn ruby_inline(r: Ruby, store: &NodeStore, nested_depth: usize) -> Inline {
    let base_inlines = r.base_emphasis.map_or_else(
        || content_range_to_inlines(r.base, store, nested_depth),
        |attr| {
            vec![format_inline(
                ForwardFormat {
                    attr,
                    target: r.base,
                    origin: ForwardOrigin::SelfContained,
                    payload: ForwardPayload::None,
                },
                store,
                nested_depth,
            )]
        },
    );
    let reading_inlines = content_range_to_inlines(r.reading, store, nested_depth);
    let inner = vec![
        Inline::Span(class_attr("ruby-base"), base_inlines),
        Inline::Span(class_attr("ruby-reading"), reading_inlines),
    ];
    Inline::Span(
        class_attr_kv(
            "ruby",
            vec![(
                "side".to_owned(),
                match r.side {
                    RubySide::Right => "right",
                    RubySide::Left => "left",
                }
                .to_owned(),
            )],
        ),
        inner,
    )
}

fn side_note_inline(s: MarginNote, store: &NodeStore, nested_depth: usize) -> Inline {
    let base_inlines = content_range_to_inlines(s.base, store, nested_depth);
    let note_inlines = content_range_to_inlines(s.note, store, nested_depth);
    let inner = vec![
        Inline::Span(class_attr("sidenote-base"), base_inlines),
        Inline::Span(class_attr("sidenote-note"), note_inlines),
    ];
    Inline::Span(
        class_attr_kv(
            "sidenote",
            vec![(
                "kind".to_owned(),
                match s.kind {
                    MarginNoteKind::Gloss => "gloss",
                    MarginNoteKind::Marginal => "marginal",
                }
                .to_owned(),
            )],
        ),
        inner,
    )
}

/// Project a forward-reference emphasis node to its Pandoc inline.
///
/// Each `ForwardAttr` maps to the closest native Pandoc construct — 太字 →
/// [`Inline::Strong`], 斜体 → [`Inline::Emph`], 上付き / 下付き小文字 →
/// [`Inline::Superscript`] / [`Inline::Subscript`] — and every attribute with
/// no native equivalent (傍点 / 縦中横 / font size / 囲み / 小書き / accent / …)
/// to a classed [`Inline::Span`] carrying the structured data as key/value
/// attributes. Both cases resolve the decorated run's real text from
/// `f.target`, so no emphasis ever discards its content.
///
/// The match is exhaustive (no `_` arm): a new [`ForwardAttr`] variant is
/// compiler-flagged here rather than silently falling through to an empty or
/// debug placeholder.
fn format_inline(f: ForwardFormat, store: &NodeStore, nested_depth: usize) -> Inline {
    let target = format_target_inlines(f, store, nested_depth);
    match f.attr {
        ForwardAttr::Bold => Inline::Strong(target),
        ForwardAttr::Italic => Inline::Emph(target),
        ForwardAttr::SuperScript => Inline::Superscript(target),
        ForwardAttr::SubScript => Inline::Subscript(target),
        // ゴシック体 is a typeface, not a weight; Pandoc has no native gothic, so
        // it stays a classed span distinct from 太字's `Strong`.
        ForwardAttr::Gothic => Inline::Span(class_attr("gothic"), target),
        ForwardAttr::Bouten { kind, position } => Inline::Span(
            class_attr_kv(
                "bouten",
                vec![
                    (
                        "kind".to_owned(),
                        roman_slug(kind.keyword()).unwrap_or("unknown").to_owned(),
                    ),
                    (
                        "position".to_owned(),
                        bouten_position_slug(position).to_owned(),
                    ),
                ],
            ),
            target,
        ),
        ForwardAttr::CombineUpright => Inline::Span(class_attr("tate-chu-yoko"), target),
        ForwardAttr::SmallScript(position) => Inline::Span(
            class_attr_kv(
                "small-script",
                vec![(
                    "position".to_owned(),
                    bouten_position_slug(position).to_owned(),
                )],
            ),
            target,
        ),
        ForwardAttr::Framed(kind) => Inline::Span(
            class_attr_kv(
                "enclosure",
                vec![("kind".to_owned(), enclosure_kind_slug(kind).to_owned())],
            ),
            target,
        ),
        ForwardAttr::Horizontal => Inline::Span(class_attr("horizontal"), target),
        ForwardAttr::Caption => Inline::Span(class_attr("caption"), target),
        // 文字サイズ carries a signed magnitude: the class names the direction
        // (larger / smaller) and the `steps` kv the stage count.
        ForwardAttr::FontSize(shift) => Inline::Span(
            class_attr_kv(
                font_size_class(shift),
                vec![("steps".to_owned(), shift.magnitude().to_string())],
            ),
            target,
        ),
        ForwardAttr::FontSizeAbsolute(size) => Inline::Span(
            class_attr_kv(
                "font-absolute",
                vec![("size".to_owned(), absolute_size_slug(size).to_owned())],
            ),
            target,
        ),
        ForwardAttr::Fraction => Inline::Span(class_attr("fraction"), target),
        ForwardAttr::AccentDot => Inline::Span(class_attr("accent-dot"), target),
        ForwardAttr::Accent(mark) => Inline::Span(
            class_attr_kv(
                "accent",
                vec![("mark".to_owned(), accent_mark_slug(mark).to_owned())],
            ),
            target,
        ),
        ForwardAttr::AlignEnd { offset } => Inline::Span(
            class_attr_kv("align-end", vec![("offset".to_owned(), offset.to_string())]),
            target,
        ),
    }
}

fn format_target_inlines(f: ForwardFormat, store: &NodeStore, nested_depth: usize) -> Vec<Inline> {
    if matches!(f.payload, ForwardPayload::NestedSource)
        && let Some(source) = store.content_range_as_plain(f.target)
        && nested_depth < MAX_NESTED_SOURCE_DEPTH
        && let Some(inlines) = nested_source_to_inlines(source, nested_depth + 1)
    {
        return inlines;
    }

    match (f.attr, f.payload, store.content_range_as_plain(f.target)) {
        (ForwardAttr::AccentDot, ForwardPayload::AccentBody(body), Some(run)) => {
            compose_accent_dots(run, store.resolve_str(body)).map_or_else(
                || content_range_to_inlines(f.target, store, nested_depth),
                |text| vec![Inline::Str(text)],
            )
        }
        (ForwardAttr::Accent(mark), _, Some(run)) => compose_single_accent(run, mark).map_or_else(
            || content_range_to_inlines(f.target, store, nested_depth),
            |glyph| vec![Inline::Str(glyph.to_string())],
        ),
        _ => content_range_to_inlines(f.target, store, nested_depth),
    }
}

fn compose_single_accent(run: &str, mark: AccentMark) -> Option<char> {
    let mut characters = run.chars();
    let letter = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    compose_accent(letter, mark)
}

fn nested_source_to_inlines(source: &str, nested_depth: usize) -> Option<Vec<Inline>> {
    let output = lex(source);
    if output.source_nodes.iter().any(|entry| match entry.node {
        NodeRef::Inline(_) => false,
        NodeRef::BlockLeaf(_) | NodeRef::BlockOpen(_) | NodeRef::BlockClose(_) => true,
    }) {
        return None;
    }
    let mut converter = Converter::inline_source(
        &output.sanitized,
        &output.source_nodes,
        &output.store,
        nested_depth,
    );
    converter.run();
    flatten_blocks(converter.blocks)
}

fn flatten_blocks(blocks: Vec<Block>) -> Option<Vec<Inline>> {
    let mut output = Vec::new();
    for block in blocks {
        let mut next = match block {
            Block::Plain(inlines) | Block::Para(inlines) | Block::Header(_, _, inlines) => inlines,
            Block::Div(_, blocks) => flatten_blocks(blocks)?,
            _ => return None,
        };
        if !output.is_empty() && !next.is_empty() {
            output.push(Inline::SoftBreak);
        }
        output.append(&mut next);
    }
    Some(output)
}

fn bouten_position_slug(p: BoutenPosition) -> &'static str {
    match p {
        BoutenPosition::Right => "right",
        BoutenPosition::Left => "left",
        BoutenPosition::Both => "both",
    }
}

/// `aozora-font-larger` / `aozora-font-smaller` — the class body for a relative
/// [`FontShift`], keyed on its sign (the magnitude rides a `steps` kv).
fn font_size_class(shift: FontShift) -> &'static str {
    if shift.larger() {
        "font-larger"
    } else {
        "font-smaller"
    }
}

fn enclosure_kind_slug(kind: EnclosureKind) -> &'static str {
    match kind {
        EnclosureKind::Rule => "rule",
        EnclosureKind::Box => "box",
        EnclosureKind::Circle => "circle",
        EnclosureKind::CircleDotted => "circle-dotted",
        EnclosureKind::DoubleRule => "double-rule",
    }
}

fn absolute_size_slug(size: AbsoluteSize) -> &'static str {
    match size {
        AbsoluteSize::ExtraLarge => "extra-large",
        AbsoluteSize::Large => "large",
        AbsoluteSize::Medium => "medium",
        AbsoluteSize::Small => "small",
    }
}

fn accent_mark_slug(mark: AccentMark) -> &'static str {
    match mark {
        AccentMark::Acute => "acute",
        AccentMark::Umlaut => "umlaut",
        AccentMark::Grave => "grave",
    }
}

fn gaiji_inline(g: Gaiji, store: &NodeStore) -> Inline {
    let mut kvs = vec![(
        "description".to_owned(),
        store.resolve_str(g.hint).to_owned(),
    )];
    kvs.push(("standalone".to_owned(), g.standalone.to_string()));
    kvs.push((
        "mencode-separator".to_owned(),
        g.mencode_separator.to_string(),
    ));
    if g.canonical.has_mencode() {
        let mut mencode = String::new();
        g.canonical
            .write_mencode(store, &mut mencode)
            .expect("write_mencode into String is infallible");
        kvs.push(("mencode".to_owned(), mencode));
    }
    let resolved = g.resolve(store).map(|value| {
        let mut text = String::new();
        value
            .write_to(&mut text)
            .expect("write resolved gaiji into String");
        text
    });
    if let Some(text) = &resolved {
        let codepoints = text
            .chars()
            .map(|character| format!("U+{:04X}", character as u32))
            .collect::<Vec<_>>()
            .join(" ");
        kvs.push(("codepoint".to_owned(), codepoints));
    }
    let inner = resolved.map_or_else(
        || vec![Inline::Str("〓".to_owned())],
        |text| vec![Inline::Str(text)],
    );
    Inline::Span(class_attr_kv("gaiji", kvs), inner)
}

/// Project a single-line layout directive.
fn line_inline(lf: LineFormat) -> Inline {
    let attr = match lf {
        LineFormat::Indent { amount, end_offset } => {
            let mut kvs = vec![("amount".to_owned(), amount.to_string())];
            if let Some(offset) = end_offset {
                kvs.push(("offset".to_owned(), offset.to_string()));
            }
            class_attr_kv("indent", kvs)
        }
        LineFormat::AlignEnd { offset } => {
            class_attr_kv("align-end", vec![("offset".to_owned(), offset.to_string())])
        }
        LineFormat::Center { page } => {
            class_attr_kv("center", vec![("page".to_owned(), page.to_string())])
        }
        LineFormat::Gothic => class_attr("line-gothic"),
        LineFormat::FontSizeAbsolute { size, bold } => class_attr_kv(
            "line-font-absolute",
            vec![
                ("size".to_owned(), absolute_size_slug(size).to_owned()),
                ("bold".to_owned(), bold.to_string()),
            ],
        ),
    };
    Inline::Span(attr, Vec::new())
}

fn annotation_inline(a: Directive, store: &NodeStore) -> Inline {
    let raw = store.resolve_str(a.raw);
    let content = match a.kind {
        DirectiveKind::EditorNote => {
            let number = raw
                .strip_prefix("［＃入力者注(")
                .and_then(|rest| rest.strip_suffix(")］"))
                .unwrap_or(raw);
            vec![Inline::Str(format!("注{number}"))]
        }
        DirectiveKind::RubyAttached | DirectiveKind::RubyRetarget => {
            vec![Inline::Str("ルビ".to_owned())]
        }
        DirectiveKind::RubyPairOpen => vec![Inline::Str("左ルビ".to_owned())],
        DirectiveKind::RubyPairClose => {
            let reading = raw
                .strip_prefix("［＃左に「")
                .and_then(|rest| rest.strip_suffix("」のルビ付き終わり］"))
                .unwrap_or(raw);
            vec![Inline::Str(format!("左ルビ「{reading}」"))]
        }
        DirectiveKind::MarginNotePairOpen => vec![Inline::Str(
            if raw.starts_with("［＃左に") {
                "左注記"
            } else {
                "注記"
            }
            .to_owned(),
        )],
        DirectiveKind::MarginNotePairClose => {
            let (label, prefix) = if raw.starts_with("［＃左に「") {
                ("左注記", "［＃左に「")
            } else {
                ("注記", "［＃「")
            };
            let note = raw
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix("」の注記付き終わり］"))
                .unwrap_or(raw);
            vec![Inline::Str(format!("{label}「{note}」"))]
        }
        DirectiveKind::NonCanonical
        | DirectiveKind::Editorial
        | DirectiveKind::Sic
        | DirectiveKind::BaseTextVariant
        | DirectiveKind::WarichuOpen
        | DirectiveKind::WarichuClose
        | DirectiveKind::Empty => Vec::new(),
    };
    Inline::Span(
        class_attr_kv(
            "annotation",
            vec![
                ("kind".to_owned(), annotation_kind_slug(a.kind).to_owned()),
                ("raw".to_owned(), raw.to_owned()),
            ],
        ),
        content,
    )
}

fn annotation_kind_slug(k: DirectiveKind) -> &'static str {
    match k {
        DirectiveKind::NonCanonical => "non-canonical",
        DirectiveKind::Editorial => "editorial",
        DirectiveKind::Sic => "sic",
        DirectiveKind::BaseTextVariant => "base-text-variant",
        DirectiveKind::WarichuOpen => "warichu-open",
        DirectiveKind::WarichuClose => "warichu-close",
        DirectiveKind::Empty => "empty",
        DirectiveKind::EditorNote => "editor-note",
        DirectiveKind::RubyAttached => "ruby-attached",
        DirectiveKind::RubyRetarget => "ruby-retarget",
        DirectiveKind::RubyPairOpen => "ruby-pair-open",
        DirectiveKind::RubyPairClose => "ruby-pair-close",
        DirectiveKind::MarginNotePairOpen => "margin-note-pair-open",
        DirectiveKind::MarginNotePairClose => "margin-note-pair-close",
    }
}

fn kaeriten_inline(k: Kaeriten, store: &NodeStore) -> Inline {
    let mark = store.resolve_str(k.mark).to_owned();
    Inline::Span(
        class_attr_kv("kaeriten", vec![("mark".to_owned(), mark.clone())]),
        vec![Inline::Str(mark)],
    )
}

fn angle_quote_inline(d: AngleQuote, store: &NodeStore, nested_depth: usize) -> Inline {
    let mut content = vec![Inline::Str("《".to_owned())];
    content.extend(content_range_to_inlines(d.content, store, nested_depth));
    content.push(Inline::Str("》".to_owned()));
    Inline::Span(class_attr("angle-quote"), content)
}

fn heading_hint_inline(h: HeadingHint, store: &NodeStore) -> Inline {
    let target = store.resolve_str(h.target).to_owned();
    // A self-contained (no-referent) hint shows its quoted target as the heading
    // text; a referent-present hint stays an empty marker.
    let content = if h.self_contained {
        vec![Inline::Str(target.clone())]
    } else {
        Vec::new()
    };
    let mut kvs = vec![
        ("level".to_owned(), h.level.outline_level().to_string()),
        ("target".to_owned(), target),
    ];
    if let Some(style) = heading_style_slug(h.style) {
        kvs.push(("style".to_owned(), style.to_owned()));
    }
    Inline::Span(class_attr_kv("heading-hint", kvs), content)
}

fn section_break_block(k: SectionKind) -> Block {
    let slug = roman_slug(k.keyword()).unwrap_or("other");
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

fn aozora_heading_block(h: Heading, store: &NodeStore, nested_depth: usize) -> Block {
    let level: i64 = match h.kind {
        HeadingKind::Large => 1,
        HeadingKind::Medium => 2,
        HeadingKind::Small => 3,
    };
    // `kind` (level) is always carried; `style` only for a non-standard
    // style, so a standard heading's projection is unchanged.
    let mut kv = vec![("kind".to_owned(), heading_kind_slug(h.kind).to_owned())];
    if let Some(style) = heading_style_slug(h.style) {
        kv.push(("style".to_owned(), style.to_owned()));
    }
    Block::Header(
        level,
        class_attr_kv("heading", kv),
        content_range_to_inlines(h.text, store, nested_depth),
    )
}

fn heading_kind_slug(k: HeadingKind) -> &'static str {
    match k {
        HeadingKind::Large => "large",
        HeadingKind::Medium => "medium",
        HeadingKind::Small => "small",
    }
}

/// Style modifier slug, or `None` for the standard style (which adds no
/// `style` attribute, keeping a standard heading's projection unchanged).
fn heading_style_slug(s: HeadingStyle) -> Option<&'static str> {
    match s {
        HeadingStyle::SameLine => Some("same-line"),
        HeadingStyle::Window => Some("window"),
        HeadingStyle::Standard => None,
    }
}

fn sashie_block(s: Illustration, store: &NodeStore, nested_depth: usize) -> Block {
    let caption = s.caption.map_or_else(Vec::new, |content| {
        vec![Block::Plain(nested_content_to_inlines(
            content,
            store,
            nested_depth,
        ))]
    });
    Block::Figure(
        illustration_attr(s, store),
        (None, caption),
        vec![Block::Plain(vec![sashie_inline(s, store, nested_depth)])],
    )
}

fn sashie_inline(s: Illustration, store: &NodeStore, nested_depth: usize) -> Inline {
    // The general form's leading description is the alt; otherwise the
    // keyword 挿絵 form's trailing 「caption」 is the next-best alt text.
    let alt = s.description.map_or_else(
        || {
            s.caption
                .map(|c| nested_content_to_inlines(c, store, nested_depth))
                .unwrap_or_default()
        },
        |description| vec![Inline::Str(store.resolve_str(description).to_owned())],
    );
    let target = (store.resolve_str(s.file).to_owned(), String::new());
    Inline::Image(illustration_attr(s, store), alt, target)
}

fn illustration_attr(s: Illustration, store: &NodeStore) -> Attr {
    let mut kvs = Vec::new();
    if let Some(number) = s.number {
        kvs.push(("number".to_owned(), store.resolve_str(number).to_owned()));
    }
    if let Some(dimensions) = s.dimensions {
        kvs.push((
            "dimensions".to_owned(),
            store.resolve_str(dimensions).to_owned(),
        ));
    }
    class_attr_kv("illustration", kvs)
}

fn inline_container(kind: InlineContainer, inlines: Vec<Inline>) -> Inline {
    match kind {
        InlineContainer::Region(region) => region_inline(region, inlines),
        InlineContainer::Warichu => Inline::Span(class_attr("warichu"), inlines),
    }
}

fn region_inline(kind: RegionFormat, inlines: Vec<Inline>) -> Inline {
    let content = match kind {
        RegionFormat::Bold { .. } => vec![Inline::Strong(inlines)],
        RegionFormat::Italic { .. } => vec![Inline::Emph(inlines)],
        _ => inlines,
    };
    Inline::Span(container_attr(kind), content)
}

fn region_block(kind: RegionFormat, blocks: Vec<Block>) -> Block {
    if let RegionFormat::Heading { level, .. } = kind
        && blocks.iter().all(|block| matches!(block, Block::Para(_)))
    {
        let mut inlines = Vec::new();
        for block in blocks {
            let Block::Para(mut paragraph) = block else {
                unreachable!("the heading block shape was checked above")
            };
            if !inlines.is_empty() && !paragraph.is_empty() {
                inlines.push(Inline::SoftBreak);
            }
            inlines.append(&mut paragraph);
        }
        return Block::Header(
            i64::from(level.outline_level()),
            container_attr(kind),
            inlines,
        );
    }
    Block::Div(container_attr(kind), blocks)
}

fn container_attr(kind: RegionFormat) -> Attr {
    let (slug, kvs): (&str, Vec<(String, String)>) = match kind {
        RegionFormat::Indent(indent) => return indent_container_attr(indent),
        RegionFormat::Warichu => ("container-warichu", Vec::new()),
        RegionFormat::Framed(enclosure) => (
            "container-keigakomi",
            vec![("kind".to_owned(), enclosure_kind_slug(enclosure).to_owned())],
        ),
        RegionFormat::AlignEnd { offset } => (
            "container-align-end",
            vec![("offset".to_owned(), offset.to_string())],
        ),
        RegionFormat::LineWidth(width) => (
            "container-line-width",
            vec![("width".to_owned(), width.0.to_string())],
        ),
        RegionFormat::Bouten { kind, position } => return bouten_container_attr(kind, position),
        RegionFormat::Bold { padded } => (
            "container-bold",
            vec![("padded".to_owned(), padded.to_string())],
        ),
        RegionFormat::Gothic { padded } => (
            "container-gothic",
            vec![("padded".to_owned(), padded.to_string())],
        ),
        RegionFormat::Italic { padded } => (
            "container-italic",
            vec![("padded".to_owned(), padded.to_string())],
        ),
        RegionFormat::Heading {
            level,
            style,
            padded,
        } => return heading_container_attr(level, style, padded),
        RegionFormat::Columns(count) => (
            "container-columns",
            vec![("count".to_owned(), count.0.to_string())],
        ),
        RegionFormat::Table => ("container-table", Vec::new()),
        RegionFormat::Horizontal => ("container-horizontal", Vec::new()),
        RegionFormat::FontSize(shift) => return font_size_container_attr(shift),
        RegionFormat::SmallScript(position) => (
            "container-small-script",
            vec![(
                "position".to_owned(),
                bouten_position_slug(position).to_owned(),
            )],
        ),
        RegionFormat::Caption { padded } => (
            "container-caption",
            vec![("padded".to_owned(), padded.to_string())],
        ),
    };
    (
        String::new(),
        vec![format!("{AOZORA_CLASS_PREFIX}{slug}")],
        kvs,
    )
}

fn indent_container_attr(indent: IndentBlock) -> Attr {
    let IndentBlock {
        amount,
        wrap,
        center,
        layout,
        styles,
    } = indent;
    let mut kvs = vec![("amount".to_owned(), amount.to_string())];
    if let Some(wrap) = wrap {
        kvs.push(("wrap".to_owned(), wrap.to_string()));
    }
    if center {
        kvs.push(("center".to_owned(), "true".to_owned()));
    }
    match layout {
        IndentLayout::Kumi(kumi) => {
            kvs.push(("kumi-lines".to_owned(), kumi.lines.to_string()));
            kvs.push(("kumi-width".to_owned(), kumi.width.to_string()));
        }
        IndentLayout::LineWidth(width) => {
            kvs.push(("width".to_owned(), width.0.to_string()));
        }
        IndentLayout::None => {}
    }
    let modifiers = styles
        .iter_formats()
        .map(Format::as_json_tag)
        .collect::<Vec<_>>()
        .join(" ");
    if !modifiers.is_empty() {
        kvs.push(("modifiers".to_owned(), modifiers));
    }
    class_attr_kv("container-indent", kvs)
}

fn bouten_container_attr(kind: BoutenKind, position: BoutenPosition) -> Attr {
    class_attr_kv(
        "container-bouten",
        vec![
            (
                "variant".to_owned(),
                roman_slug(kind.keyword()).unwrap_or("unknown").to_owned(),
            ),
            (
                "position".to_owned(),
                bouten_position_slug(position).to_owned(),
            ),
        ],
    )
}

fn heading_container_attr(level: HeadingKind, style: HeadingStyle, padded: bool) -> Attr {
    class_attr_kv(
        "container-heading",
        vec![
            ("level".to_owned(), heading_kind_slug(level).to_owned()),
            (
                "style".to_owned(),
                heading_style_slug(style).unwrap_or("standard").to_owned(),
            ),
            ("padded".to_owned(), padded.to_string()),
        ],
    )
}

fn font_size_container_attr(shift: FontShift) -> Attr {
    class_attr_kv(
        "container-font-size",
        vec![
            (
                "direction".to_owned(),
                if shift.larger() { "larger" } else { "smaller" }.to_owned(),
            ),
            ("steps".to_owned(), shift.magnitude().to_string()),
        ],
    )
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use crate::syntax::BlockStyles;
    use crate::syntax::ast::GaijiCanonicalOwned;

    #[test]
    fn wire_slugs_cover_every_projected_variant() {
        for (kind, expected) in [
            (EnclosureKind::Rule, "rule"),
            (EnclosureKind::Box, "box"),
            (EnclosureKind::Circle, "circle"),
            (EnclosureKind::CircleDotted, "circle-dotted"),
            (EnclosureKind::DoubleRule, "double-rule"),
        ] {
            assert_eq!(enclosure_kind_slug(kind), expected);
        }
        for (size, expected) in [
            (AbsoluteSize::ExtraLarge, "extra-large"),
            (AbsoluteSize::Large, "large"),
            (AbsoluteSize::Medium, "medium"),
            (AbsoluteSize::Small, "small"),
        ] {
            assert_eq!(absolute_size_slug(size), expected);
        }
        for (mark, expected) in [
            (AccentMark::Acute, "acute"),
            (AccentMark::Umlaut, "umlaut"),
            (AccentMark::Grave, "grave"),
        ] {
            assert_eq!(accent_mark_slug(mark), expected);
        }
    }

    /// Plain text round-trips into a single Pandoc Para of `Inline::Str`.
    #[test]
    fn plain_text_becomes_para() {
        let doc = Document::new("Hello, world.");
        let pandoc = to_pandoc(&doc.snapshot());
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
        let pandoc = to_pandoc(&doc.snapshot());
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
        assert_eq!(
            &pandoc.blocks[1],
            &Block::Para(vec![Inline::Str("Two.".to_owned())])
        );
    }

    #[test]
    fn single_newline_is_one_soft_break() {
        assert_eq!(
            project("One\nTwo"),
            vec![Block::Para(vec![
                Inline::Str("One".to_owned()),
                Inline::SoftBreak,
                Inline::Str("Two".to_owned()),
            ])]
        );
    }

    #[test]
    fn triple_newline_has_no_leading_break_in_second_paragraph() {
        assert_eq!(
            project("One\n\n\nTwo"),
            vec![
                Block::Para(vec![Inline::Str("One".to_owned())]),
                Block::Para(vec![Inline::Str("Two".to_owned())]),
            ]
        );
    }

    #[test]
    fn trailing_newlines_do_not_emit_breaks_or_empty_paragraphs() {
        for source in ["One\n", "One\n\n", "One\n\n\n"] {
            assert_eq!(
                project(source),
                vec![Block::Para(vec![Inline::Str("One".to_owned())])],
                "{source:?}"
            );
        }
    }

    #[test]
    fn newline_run_before_inline_node_resolves_once() {
        let single = project("One\n｜青梅《おうめ》");
        let Some(Block::Para(single)) = single.first() else {
            panic!("expected one paragraph: {single:?}")
        };
        assert!(matches!(single.get(1), Some(Inline::SoftBreak)));
        assert!(matches!(single.get(2), Some(Inline::Span(attr, _)) if has_class(attr, "ruby")));

        let double = project("One\n\n｜青梅《おうめ》");
        assert_eq!(double.len(), 2, "{double:?}");
        let Block::Para(second) = &double[1] else {
            panic!("expected second paragraph: {double:?}")
        };
        assert!(matches!(second.first(), Some(Inline::Span(attr, _)) if has_class(attr, "ruby")));
        assert!(
            !second
                .iter()
                .any(|inline| matches!(inline, Inline::SoftBreak))
        );
    }

    #[test]
    fn newline_after_inline_node_is_not_lost() {
        let blocks = project("｜青梅《おうめ》\nTwo");
        let Some(Block::Para(inlines)) = blocks.first() else {
            panic!("expected one paragraph: {blocks:?}")
        };
        assert!(matches!(inlines.first(), Some(Inline::Span(attr, _)) if has_class(attr, "ruby")));
        assert!(matches!(inlines.get(1), Some(Inline::SoftBreak)));
        assert_eq!(inlines.get(2), Some(&Inline::Str("Two".to_owned())));
    }

    #[test]
    fn newline_is_not_dropped_when_new_inline_frame_is_empty() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.push_inline(Inline::Str("before".to_owned()));
        converter.open_container(RegionFormat::Caption { padded: false });
        converter.pending_newlines = 1;
        converter.resolve_pending_newlines();
        converter.push_inline(Inline::Str("after".to_owned()));
        converter.close_container(true);
        converter.run();

        let [Block::Para(inlines)] = converter.blocks.as_slice() else {
            panic!("expected one paragraph: {:?}", converter.blocks)
        };
        assert_eq!(inlines.first(), Some(&Inline::Str("before".to_owned())));
        let Some(Inline::Span(_, content)) = inlines.get(1) else {
            panic!("expected caption span: {inlines:?}")
        };
        assert!(matches!(content.first(), Some(Inline::SoftBreak)));
        assert_eq!(content.get(1), Some(&Inline::Str("after".to_owned())));
    }

    #[test]
    fn emitted_inline_frame_materializes_each_paragraph_before_a_block() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Caption { padded: false });

        converter.push_inline(Inline::Str("first".to_owned()));
        converter.flush_paragraph();
        converter.push_inline(Inline::Str("second".to_owned()));
        converter.flush_paragraph();
        converter.push_block(Block::HorizontalRule);
        converter.close_container(true);
        converter.run();

        let [
            Block::Para(first),
            Block::Para(second),
            Block::HorizontalRule,
        ] = converter.blocks.as_slice()
        else {
            panic!(
                "expected both caption fragments before the block: {:?}",
                converter.blocks
            )
        };
        let [Inline::Span(_, first)] = first.as_slice() else {
            panic!("expected first caption fragment: {first:?}")
        };
        let [Inline::Span(_, second)] = second.as_slice() else {
            panic!("expected second caption fragment: {second:?}")
        };
        assert_eq!(first, &[Inline::Str("first".to_owned())]);
        assert_eq!(second, &[Inline::Str("second".to_owned())]);
    }

    /// Ruby with explicit delimiter projects to a Span.aozora-ruby
    /// carrying base / reading sub-spans.
    #[test]
    fn ruby_projects_to_span() {
        let doc = Document::new("｜青梅《おうめ》");
        let pandoc = to_pandoc(&doc.snapshot());
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
        let pandoc = to_pandoc(&doc.snapshot());
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
        let pandoc = to_pandoc(&doc.snapshot());
        let has_indent_div = pandoc.blocks.iter().any(|b| {
            matches!(
                b,
                Block::Div(attr, _)
                    if attr.1.iter().any(|c| c.contains("aozora-container-indent"))
            )
        });
        assert!(has_indent_div, "no indent Div: {:?}", pandoc.blocks);
    }

    // -----------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------

    /// Project `src` through the full pipeline and return the doc blocks.
    fn project(src: &str) -> Vec<Block> {
        let doc = Document::new(src);
        to_pandoc(&doc.snapshot()).blocks
    }

    /// Whether any class in `attr` ends with `suffix` (the `aozora-`
    /// prefix is constant; matching the tail keeps assertions stable).
    fn has_class(attr: &Attr, suffix: &str) -> bool {
        attr.1.iter().any(|c| c == &format!("aozora-{suffix}"))
    }

    /// Look up a key in an `Attr`'s key/value list.
    fn kv<'a>(attr: &'a Attr, key: &str) -> Option<&'a str> {
        attr.2
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Find the first inline `Span` (recursively) whose class list carries
    /// `aozora-{suffix}`. Walks into nested inlines so a span buried in a
    /// `Para` is found.
    fn find_span<'a>(blocks: &'a [Block], suffix: &str) -> Option<(&'a Attr, &'a [Inline])> {
        fn walk_inlines<'a>(
            inlines: &'a [Inline],
            suffix: &str,
        ) -> Option<(&'a Attr, &'a [Inline])> {
            for inline in inlines {
                if let Inline::Span(attr, inner) = inline {
                    if has_class(attr, suffix) {
                        return Some((attr, inner.as_slice()));
                    }
                    if let Some(found) = walk_inlines(inner, suffix) {
                        return Some(found);
                    }
                }
            }
            None
        }
        fn walk_blocks<'a>(blocks: &'a [Block], suffix: &str) -> Option<(&'a Attr, &'a [Inline])> {
            for block in blocks {
                let found = match block {
                    Block::Para(inlines) | Block::Header(_, _, inlines) => {
                        walk_inlines(inlines, suffix)
                    }
                    Block::Div(_, inner) => walk_blocks(inner, suffix),
                    Block::Figure(_, (_, caption), inner) => {
                        walk_blocks(caption, suffix).or_else(|| walk_blocks(inner, suffix))
                    }
                    _ => None,
                };
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        walk_blocks(blocks, suffix)
    }

    /// Find the first inline (recursively, into nested spans) satisfying
    /// `pred`. Walks `Para` / `Header` / `Div` blocks and every inline's
    /// children so a construct buried in a styled span is still found.
    fn find_inline(blocks: &[Block], pred: impl Fn(&Inline) -> bool + Copy) -> Option<&Inline> {
        fn walk_inlines(
            inlines: &[Inline],
            pred: impl Fn(&Inline) -> bool + Copy,
        ) -> Option<&Inline> {
            for inline in inlines {
                if pred(inline) {
                    return Some(inline);
                }
                let children = match inline {
                    Inline::Span(_, inner)
                    | Inline::Strong(inner)
                    | Inline::Emph(inner)
                    | Inline::Superscript(inner)
                    | Inline::Subscript(inner) => Some(inner.as_slice()),
                    _ => None,
                };
                if let Some(found) = children.and_then(|c| walk_inlines(c, pred)) {
                    return Some(found);
                }
            }
            None
        }
        fn walk_blocks(blocks: &[Block], pred: impl Fn(&Inline) -> bool + Copy) -> Option<&Inline> {
            for block in blocks {
                let found = match block {
                    Block::Para(inlines) | Block::Header(_, _, inlines) => {
                        walk_inlines(inlines, pred)
                    }
                    Block::Div(_, inner) => walk_blocks(inner, pred),
                    Block::Figure(_, (_, caption), inner) => {
                        walk_blocks(caption, pred).or_else(|| walk_blocks(inner, pred))
                    }
                    _ => None,
                };
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        walk_blocks(blocks, pred)
    }

    /// Find the first top-level `Div` whose class list carries
    /// `aozora-{suffix}`.
    fn find_div<'a>(blocks: &'a [Block], suffix: &str) -> Option<(&'a Attr, &'a [Block])> {
        blocks.iter().find_map(|b| match b {
            Block::Div(attr, inner) if has_class(attr, suffix) => Some((attr, inner.as_slice())),
            _ => None,
        })
    }

    // -----------------------------------------------------------------
    // Inline nodes (source-driven)
    // -----------------------------------------------------------------

    #[test]
    fn implicit_ruby_projects_base_and_reading() {
        let blocks = project("青梅《おうめ》という地名。\n");
        let (_, inner) = find_span(&blocks, "ruby").expect("ruby span");
        assert_eq!(inner.len(), 2, "ruby has base + reading children");
    }

    #[test]
    fn implicit_ruby_projects_gaiji_and_text_as_one_base() {
        let blocks = project("※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》");
        let (_, base) = find_span(&blocks, "ruby-base").expect("ruby base span");
        let [Inline::Span(gaiji_attr, gaiji), Inline::Str(text)] = base else {
            panic!("expected gaiji and text in one ruby base, got {base:?}");
        };
        assert!(has_class(gaiji_attr, "gaiji"));
        assert_eq!(gaiji, &[Inline::Str("犍".to_owned())]);
        assert_eq!(text, "陀多");
    }

    #[test]
    fn explicit_ruby_projects_as_ruby_span() {
        let blocks = project("｜青梅《おうめ》\n");
        let (_, inner) = find_span(&blocks, "ruby").expect("ruby span");
        assert_eq!(inner.len(), 2, "ruby has base + reading children");
    }

    #[test]
    fn ruby_base_keeps_semantic_kaeriten_segments() {
        let blocks =
            project("｜瑞岩東畔命［＃二］軽舟［＃一］《ずいがんとうはんめいずけいしうを》");
        let (_, base) = find_span(&blocks, "ruby-base").expect("ruby base span");
        let [
            Inline::Str(first),
            Inline::Span(two_attr, two),
            Inline::Str(second),
            Inline::Span(one_attr, one),
        ] = base
        else {
            panic!("expected text/kaeriten/text/kaeriten, got {base:?}");
        };
        assert_eq!(first, "瑞岩東畔命");
        assert!(has_class(two_attr, "kaeriten"));
        assert_eq!(two, &[Inline::Str("二".to_owned())]);
        assert_eq!(second, "軽舟");
        assert!(has_class(one_attr, "kaeriten"));
        assert_eq!(one, &[Inline::Str("一".to_owned())]);
    }

    #[test]
    fn ruby_reading_keeps_nested_forward_format() {
        let blocks = project("折口《ツムレ［＃「ムレ」に白丸傍点］》");
        let (_, reading) = find_span(&blocks, "ruby-reading").expect("ruby reading span");
        let [Inline::Str(prefix), Inline::Span(attr, target)] = reading else {
            panic!("expected text and bouten reading, got {reading:?}");
        };
        assert_eq!(prefix, "ツ");
        assert!(has_class(attr, "bouten"));
        assert_eq!(target, &[Inline::Str("ムレ".to_owned())]);
    }

    #[test]
    fn left_ruby_projects_as_ruby_span() {
        let blocks = project("未［＃「未」の左に「ザル」のルビ］んとす。\n");
        let (attr, inner) = find_span(&blocks, "ruby").expect("left ruby span");
        assert_eq!(kv(attr, "side"), Some("left"));
        let (_, base) = inner
            .iter()
            .find_map(|i| match i {
                Inline::Span(a, c) if has_class(a, "ruby-base") => Some((a, c)),
                _ => None,
            })
            .expect("ruby-base sub-span");
        assert_eq!(base, &[Inline::Str("未".to_owned())], "left ruby base text");
    }

    #[test]
    fn left_ruby_keeps_gaiji_in_base_and_reading() {
        for (source, field) in [
            (
                "銅※［＃「金＋拔のつくり」、第3水準1-93-6］子［＃「銅※［＃「金＋拔のつくり」、第3水準1-93-6］子」の左に「どびょうし」のルビ］",
                "ruby-base",
            ),
            (
                "未［＃「未」の左に「※［＃「特のへん＋廴＋聿」、第3水準1-87-71］」のルビ］",
                "ruby-reading",
            ),
        ] {
            let blocks = project(source);
            let (_, inlines) = find_span(&blocks, field).expect("ruby field span");
            assert!(
                inlines.iter().any(
                    |inline| matches!(inline, Inline::Span(attr, _) if has_class(attr, "gaiji"))
                ),
                "gaiji missing from {field}: {inlines:?}"
            );
        }
    }

    #[test]
    fn side_note_projects_base_and_note_subspans() {
        let blocks = project("未来［＃「未来」の左に「みらい」の注記］を見る。\n");
        let (attr, inner) = find_span(&blocks, "sidenote").expect("sidenote span");
        assert_eq!(kv(attr, "kind"), Some("gloss"));
        assert!(
            inner.iter().any(|i| matches!(
                i,
                Inline::Span(a, _) if has_class(a, "sidenote-base")
            )),
            "sidenote has base sub-span: {inner:?}"
        );
        assert!(
            inner.iter().any(|i| matches!(
                i,
                Inline::Span(a, _) if has_class(a, "sidenote-note")
            )),
            "sidenote has note sub-span: {inner:?}"
        );
    }

    #[test]
    fn marginal_note_kind_is_not_collapsed_into_gloss() {
        let blocks = project("伏字［＃「伏字」に「×」の傍記］\n");
        let (attr, _) = find_span(&blocks, "sidenote").expect("sidenote span");
        assert_eq!(kv(attr, "kind"), Some("marginal"));
    }

    #[test]
    fn side_note_keeps_a_structured_gaiji_base() {
        let blocks = project(
            "※［＃「てへん＋僉」、第3水準1-84-94］［＃「※［＃「てへん＋僉」、第3水準1-84-94］」の左に「アラタムル」の注記］",
        );
        let (_, base) = find_span(&blocks, "sidenote-base").expect("sidenote base span");
        assert!(matches!(
            base,
            [Inline::Span(attr, _)] if has_class(attr, "gaiji")
        ));
    }

    #[test]
    fn ruby_base_keeps_nested_margin_note() {
        let blocks = project("｜短尺［＃「尺」に「（冊）」の注記］《タンシヤク》");
        let (_, base) = find_span(&blocks, "ruby-base").expect("ruby base span");
        let [Inline::Str(prefix), Inline::Span(attr, _)] = base else {
            panic!("expected text and sidenote, got {base:?}");
        };
        assert_eq!(prefix, "短");
        assert!(has_class(attr, "sidenote"));
        let (_, note) = find_span(&blocks, "sidenote-note").expect("nested note span");
        assert_eq!(note, &[Inline::Str("（冊）".to_owned())]);
    }

    #[test]
    fn bouten_carries_kind_and_position() {
        let blocks = project("青空［＃「青空」に傍点］を見上げる。\n");
        let (attr, inner) = find_span(&blocks, "bouten").expect("bouten span");
        assert_eq!(kv(attr, "kind"), Some("goma"), "goma bouten kind slug");
        assert_eq!(
            kv(attr, "position"),
            Some("right"),
            "default right position"
        );
        assert_eq!(
            inner,
            &[Inline::Str("青空".to_owned())],
            "bouten target text"
        );
    }

    #[test]
    fn bouten_black_triangle_kind_slug() {
        let blocks = project("規範［＃「規範」に黒三角傍点］を説く。\n");
        let (attr, _) = find_span(&blocks, "bouten").expect("bouten span");
        assert_eq!(
            kv(attr, "kind"),
            Some("kurosankaku"),
            "black-triangle bouten slug"
        );
    }

    #[test]
    fn tate_chu_yoko_projects_to_tcy_span() {
        // The directive sits immediately after ３３, so the literal folds into
        // the node (`Reclaimed`) and the tcy span is the sole copy. (The
        // non-adjacent `明治３３年［＃…］` form splices a `Detached` decoration —
        // covered by `non_adjacent_forward_styles_referent_once`.)
        let blocks = project("明治３３［＃「３３」は縦中横］年に。\n");
        let (_, inner) = find_span(&blocks, "tate-chu-yoko").expect("tcy span");
        assert_eq!(
            inner,
            &[Inline::Str("３３".to_owned())],
            "tcy embedded text"
        );
    }

    #[test]
    fn non_adjacent_forward_styles_referent_once() {
        // #333: the non-adjacent referent 青空 is styled in place (a `Detached`
        // decoration projected as a bouten span), while the bracket stays
        // `Referenced` and projects nothing. 青空 appears exactly once — the
        // styling is added, the #231/#228 no-double-projection invariant holds.
        let blocks = project("青空の下を歩く［＃「青空」に傍点］");
        let (_, inner) = find_span(&blocks, "bouten").expect("styled referent span");
        assert_eq!(
            inner,
            &[Inline::Str("青空".to_owned())],
            "the decoration styles 青空"
        );
        let Some(Block::Para(inlines)) = blocks.first() else {
            panic!("expected a single Para, got {blocks:?}");
        };
        assert_eq!(inlines.len(), 2, "styled span + plain tail: {inlines:?}");
        assert_eq!(
            inlines[1],
            Inline::Str("の下を歩く".to_owned()),
            "the tail after the referent stays plain"
        );
    }

    #[test]
    fn resolved_gaiji_emits_resolved_char() {
        let blocks = project("珍しき木※［＃「木＋吶のつくり」、第3水準1-85-54］が立つ。\n");
        let (attr, inner) = find_span(&blocks, "gaiji").expect("gaiji span");
        assert_eq!(
            kv(attr, "description"),
            Some("木＋吶のつくり"),
            "gaiji description"
        );
        assert_eq!(kv(attr, "mencode"), Some("第3水準1-85-54"), "gaiji mencode");
        assert_eq!(kv(attr, "codepoint"), Some("U+6798"));
        assert_eq!(kv(attr, "standalone"), Some("false"));
        assert_eq!(kv(attr, "mencode-separator"), Some("true"));
        assert_eq!(inner, &[Inline::Str("枘".to_owned())]);
    }

    #[test]
    fn resolved_gaiji_emits_combining_sequence() {
        let blocks = project("※［＃「か半濁点」、第3水準1-4-87］");
        let (attr, inner) = find_span(&blocks, "gaiji").expect("gaiji span");
        assert_eq!(kv(attr, "mencode"), Some("第3水準1-4-87"));
        assert_eq!(kv(attr, "codepoint"), Some("U+304B U+309A"));
        assert_eq!(inner, &[Inline::Str("か゚".to_owned())]);
    }

    #[test]
    fn unresolved_gaiji_emits_geta_placeholder() {
        let blocks = project("※［＃「架空の外字」、第3水準99-99-99］");
        let (_, inner) = find_span(&blocks, "gaiji").expect("gaiji span");
        assert_eq!(
            inner,
            &[Inline::Str("〓".to_owned())],
            "unresolved gaiji → 〓 placeholder"
        );
    }

    #[test]
    fn gaiji_projection_retains_standalone_and_separator_provenance() {
        let mut store = NodeStore::new();
        let hint = store.intern("字形");
        let inline = gaiji_inline(
            Gaiji {
                hint,
                canonical: GaijiCanonicalOwned::Unicode('字'),
                mencode_separator: false,
                standalone: true,
            },
            &store,
        );
        let Inline::Span(attr, content) = inline else {
            panic!("gaiji projects to Span")
        };
        assert_eq!(kv(&attr, "standalone"), Some("true"));
        assert_eq!(kv(&attr, "mencode-separator"), Some("false"));
        assert_eq!(kv(&attr, "codepoint"), Some("U+5B57"));
        assert_eq!(content, vec![Inline::Str("字".to_owned())]);
    }

    #[test]
    fn angle_quote_projects_to_span() {
        let blocks = project("≪重要≫な記述。\n");
        let (_, inner) = find_span(&blocks, "angle-quote").expect("angle-quote span");
        assert_eq!(
            inner,
            &[
                Inline::Str("《".to_owned()),
                Inline::Str("重要".to_owned()),
                Inline::Str("》".to_owned()),
            ],
            "angle-quote display text"
        );
    }

    #[test]
    fn angle_quote_keeps_nested_gaiji_and_italic_target() {
        let blocks = project(
            "≪前※［＃「特のへん＋廴＋聿」、第3水準1-87-71］l'oiseau royal［＃「l'oiseau royal」は斜体］後≫",
        );
        let (_, inner) = find_span(&blocks, "angle-quote").expect("angle quote span");
        assert!(
            inner
                .iter()
                .any(|inline| matches!(inline, Inline::Span(attr, _) if has_class(attr, "gaiji")))
        );
        assert!(inner.iter().any(|inline| {
            matches!(inline, Inline::Emph(content) if content == &[Inline::Str("l'oiseau royal".to_owned())])
        }));
    }

    #[test]
    fn ruby_base_illustration_projects_as_an_inline_image() {
        let blocks = project(
            "｜［＃底本が「オム」とルビを付した梵字（fig1317_17.png、横23×縦22）入る］《オム》",
        );
        let (_, base) = find_span(&blocks, "ruby-base").expect("ruby base span");
        let [Inline::Image(attr, _, (file, _))] = base else {
            panic!("expected inline illustration, got {base:?}");
        };
        assert!(has_class(attr, "illustration"));
        assert_eq!(file, "fig1317_17.png");
    }

    #[test]
    fn kaeriten_re_mark_projects_to_span() {
        let blocks = project("天［＃（レ）］地\n");
        let (attr, inner) = find_span(&blocks, "kaeriten").expect("kaeriten span");
        assert_eq!(kv(attr, "mark"), Some("（レ）"), "kaeriten mark text");
        assert_eq!(inner, &[Inline::Str("（レ）".to_owned())]);
    }

    #[test]
    fn ruby_base_emphasis_projects_inside_the_base() {
        let blocks = project("我《われ》の名は［＃「我」に傍点］");
        let (_, inner) = find_span(&blocks, "bouten").expect("ruby-base bouten span");
        assert_eq!(inner, &[Inline::Str("我".to_owned())]);
    }

    #[test]
    fn center_page_projects_to_center_span() {
        let blocks = project("［＃ページの左右中央］題名\n");
        let (_, inner) = find_span(&blocks, "center").expect("center span");
        assert!(
            inner.is_empty(),
            "center is an empty marker span: {inner:?}"
        );
    }

    #[test]
    fn line_gothic_projects_to_semantic_marker() {
        let blocks = project("本文。［＃この行はゴシック体］\n");
        let (_, inner) = find_span(&blocks, "line-gothic").expect("line-gothic span");
        assert!(inner.is_empty(), "line marker has no inline content");
    }

    #[test]
    fn line_font_size_projects_size_and_weight() {
        let blocks = project("見出し行［＃大文字、太字］\n");
        let (attr, inner) =
            find_span(&blocks, "line-font-absolute").expect("line-font-absolute span");
        assert_eq!(kv(attr, "size"), Some("large"));
        assert_eq!(kv(attr, "bold"), Some("true"));
        assert!(inner.is_empty(), "line marker has no inline content");
    }

    #[test]
    fn heading_hint_carries_level_and_target() {
        let blocks = project("序章\n本文\n［＃「序章」は中見出し］\n");
        let (attr, _) = find_span(&blocks, "heading-hint").expect("heading-hint span");
        assert_eq!(kv(attr, "level"), Some("2"), "中見出し → level 2");
        assert_eq!(kv(attr, "target"), Some("序章"), "heading-hint target");
    }

    #[test]
    fn same_line_heading_emits_heading_hint() {
        let blocks = project("萩原朔太郎［＃「萩原朔太郎」は同行中見出し］\u{3000}二十年の友。\n");
        let (attr, _) = find_span(&blocks, "heading-hint").expect("heading-hint span");
        assert_eq!(kv(attr, "target"), Some("萩原朔太郎"), "same-line target");
        assert_eq!(kv(attr, "style"), Some("same-line"));
    }

    #[test]
    fn editorial_annotation_kind_slug() {
        let blocks = project("［＃見出し］序章［＃見出し終わり］\n");
        let (attr, _) = find_span(&blocks, "annotation").expect("annotation span");
        assert_eq!(kv(attr, "kind"), Some("editorial"), "annotation kind");
        assert!(
            kv(attr, "raw").is_some_and(|r| r.contains("見出し")),
            "annotation carries raw text"
        );
    }

    #[test]
    fn sic_annotation_kind_slug() {
        let blocks = project("そういう風［＃「いう風」はママ］だ\n");
        let (attr, _) = find_span(&blocks, "annotation").expect("annotation span");
        assert_eq!(kv(attr, "kind"), Some("sic"), "ママ → sic kind");
    }

    #[test]
    fn base_text_variant_annotation_kind_slug() {
        let blocks = project("間違い［＃「間違い」は底本では「間違ひ」］です\n");
        let (attr, _) = find_span(&blocks, "annotation").expect("annotation span");
        assert_eq!(
            kv(attr, "kind"),
            Some("base-text-variant"),
            "底本では → base-text-variant kind"
        );
    }

    #[test]
    fn visible_annotation_kinds_keep_their_reader_facing_labels() {
        for (kind, raw, expected) in [
            (DirectiveKind::EditorNote, "［＃入力者注(12)］", "注12"),
            (DirectiveKind::RubyAttached, "［＃「X」にルビ］", "ルビ"),
            (
                DirectiveKind::RubyRetarget,
                "［＃ルビは「X」にかかる］",
                "ルビ",
            ),
            (DirectiveKind::RubyPairOpen, "［＃左にルビ付き］", "左ルビ"),
            (
                DirectiveKind::RubyPairClose,
                "［＃左に「よみ」のルビ付き終わり］",
                "左ルビ「よみ」",
            ),
            (
                DirectiveKind::MarginNotePairOpen,
                "［＃左に注記付き］",
                "左注記",
            ),
            (
                DirectiveKind::MarginNotePairClose,
                "［＃「注」の注記付き終わり］",
                "注記「注」",
            ),
        ] {
            let mut store = NodeStore::new();
            let raw = store.intern(raw);
            let Inline::Span(attr, content) = annotation_inline(Directive { raw, kind }, &store)
            else {
                panic!("annotation must be a Span")
            };
            assert_eq!(kv(&attr, "kind"), Some(annotation_kind_slug(kind)));
            assert_eq!(content, vec![Inline::Str(expected.to_owned())]);
        }
    }

    #[test]
    fn hidden_annotation_kinds_have_no_visible_children() {
        for kind in [
            DirectiveKind::NonCanonical,
            DirectiveKind::Editorial,
            DirectiveKind::Sic,
            DirectiveKind::BaseTextVariant,
            DirectiveKind::Empty,
        ] {
            let mut store = NodeStore::new();
            let raw = store.intern("［＃raw］");
            let Inline::Span(_, content) = annotation_inline(Directive { raw, kind }, &store)
            else {
                panic!("annotation must be a Span")
            };
            assert!(content.is_empty(), "{kind:?}: {content:?}");
        }
    }

    #[test]
    fn warichu_annotations_form_one_structural_span() {
        let blocks = project("［＃割り注］上の段／下の段［＃割り注終わり］\n");
        let (_, inner) = find_span(&blocks, "warichu").expect("warichu span");
        assert_eq!(inner, &[Inline::Str("上の段／下の段".to_owned())]);
        assert!(find_span(&blocks, "annotation").is_none());
    }

    #[test]
    fn crossed_warichu_and_region_scopes_split_without_annotations() {
        for (source, warichu_count, bold_count) in [
            (
                "［＃割り注］［＃太字］X［＃割り注終わり］Y［＃太字終わり］",
                1,
                2,
            ),
            (
                "［＃太字］［＃割り注］X［＃太字終わり］Y［＃割り注終わり］",
                2,
                1,
            ),
        ] {
            let json = serde_json::to_string(&project(source)).expect("serialize blocks");
            assert_eq!(
                json.matches("aozora-warichu").count(),
                warichu_count,
                "{json}"
            );
            assert_eq!(
                json.matches("aozora-container-bold").count(),
                bold_count,
                "{json}"
            );
            assert!(!json.contains("warichu-close"), "{json}");
            assert_eq!(json.matches("\"X\"").count(), 1, "{json}");
            assert_eq!(json.matches("\"Y\"").count(), 1, "{json}");
        }
    }

    // -----------------------------------------------------------------
    // Forward-reference emphasis projection (WS-5)
    // -----------------------------------------------------------------

    #[test]
    fn emphasis_bold_projects_to_strong() {
        let blocks = project("甲［＃「甲」は太字］\n");
        let strong = find_inline(&blocks, |i| matches!(i, Inline::Strong(_)))
            .expect("bold projects to a Strong inline");
        assert_eq!(
            strong,
            &Inline::Strong(vec![Inline::Str("甲".to_owned())]),
            "Strong carries the real target text"
        );
    }

    #[test]
    fn bouten_projects_structured_gaiji_target_once() {
        let blocks =
            project("※［＃「木＋吶のつくり」、第3水準1-85-54］陀多［＃「枘陀多」に傍点］\n");
        let (_, inner) = find_span(&blocks, "bouten").expect("bouten span");
        assert_eq!(inner.len(), 2);
        assert!(matches!(
            &inner[0],
            Inline::Span(attr, glyph)
                if has_class(attr, "gaiji") && glyph == &[Inline::Str("枘".to_owned())]
        ));
        assert_eq!(inner[1], Inline::Str("陀多".to_owned()));
        let json = serde_json::to_string(&blocks).expect("serialize blocks");
        assert_eq!(json.matches("枘").count(), 1);
    }

    #[test]
    fn emphasis_italic_projects_to_emph() {
        let blocks = project("乙［＃「乙」は斜体］\n");
        let emph = find_inline(&blocks, |i| matches!(i, Inline::Emph(_)))
            .expect("italic projects to an Emph inline");
        assert_eq!(
            emph,
            &Inline::Emph(vec![Inline::Str("乙".to_owned())]),
            "Emph carries the real target text"
        );
    }

    #[test]
    fn emphasis_font_size_projects_to_font_span() {
        let blocks = project("甲［＃「甲」は2段階大きな文字］\n");
        let (attr, inner) = find_span(&blocks, "font-larger").expect("font-larger span");
        assert_eq!(kv(attr, "steps"), Some("2"), "font shift magnitude");
        assert_eq!(
            inner,
            &[Inline::Str("甲".to_owned())],
            "font-size span carries the real target text, not a Debug dump"
        );
    }

    #[test]
    fn emphasis_superscript_projects_to_superscript() {
        let blocks = project("ｅ２［＃「２」は上付き小文字］\n");
        let sup = find_inline(&blocks, |i| matches!(i, Inline::Superscript(_)))
            .expect("superscript projects to a Superscript inline");
        assert_eq!(
            sup,
            &Inline::Superscript(vec![Inline::Str("２".to_owned())]),
            "Superscript carries the real target text"
        );
    }

    #[test]
    fn emphasis_subscript_projects_to_subscript() {
        let blocks = project("ｅ２［＃「２」は下付き小文字］\n");
        let sub = find_inline(&blocks, |i| matches!(i, Inline::Subscript(_)))
            .expect("subscript projects to a Subscript inline");
        assert_eq!(
            sub,
            &Inline::Subscript(vec![Inline::Str("２".to_owned())]),
            "Subscript carries the real target text"
        );
    }

    /// The whole point of WS-5: a document mixing bold / italic / font-size /
    /// superscript projects to the native Pandoc constructs carrying the real
    /// text, and no `ForwardFormat` Debug dump survives anywhere in the JSON.
    #[test]
    fn mixed_emphasis_projection_carries_text_and_drops_debug() {
        let blocks = project(
            "甲［＃「甲」は太字］\n乙［＃「乙」は斜体］\n\
             丙［＃「丙」は2段階大きな文字］\nｅ２［＃「２」は上付き小文字］\n",
        );
        let json = serde_json::to_string(&blocks).expect("serialise blocks");
        for tag in ["Strong", "Emph", "Superscript"] {
            assert!(
                json.contains(tag),
                "expected a {tag} inline in the projection: {json}"
            );
        }
        for text in ["甲", "乙", "丙", "２"] {
            assert!(
                json.contains(text),
                "target text {text} must survive: {json}"
            );
        }
        assert!(
            !json.contains("ForwardFormat"),
            "no ForwardFormat Debug dump may remain: {json}"
        );
    }

    /// Compile-time exhaustiveness guard. Every [`ForwardAttr`] and [`Segment`]
    /// variant must be matched with no `_` fall-through, so adding a variant
    /// breaks this test's build — forcing a deliberate projection rather than a
    /// silent empty `Str` / debug span. The runtime assertion then pins that the
    /// live [`format_inline`] never emits an empty-class placeholder for any
    /// forward attribute.
    #[test]
    fn forward_attr_and_segment_projection_is_exhaustive() {
        // Naming the discriminants with no wildcard is the guard; the body is
        // irrelevant.
        fn cover_forward_attr(a: ForwardAttr) {
            match a {
                ForwardAttr::Bold
                | ForwardAttr::Gothic
                | ForwardAttr::Italic
                | ForwardAttr::SuperScript
                | ForwardAttr::SubScript
                | ForwardAttr::SmallScript(_)
                | ForwardAttr::Framed(_)
                | ForwardAttr::Horizontal
                | ForwardAttr::Caption
                | ForwardAttr::FontSize(_)
                | ForwardAttr::FontSizeAbsolute(_)
                | ForwardAttr::Bouten { .. }
                | ForwardAttr::CombineUpright
                | ForwardAttr::Fraction
                | ForwardAttr::AccentDot
                | ForwardAttr::Accent(_)
                | ForwardAttr::AlignEnd { .. } => {}
            }
        }
        fn cover_segment(s: Segment) {
            match s {
                Segment::Text(_) | Segment::Gaiji(_) | Segment::Directive(_) | Segment::Node(_) => {
                }
            }
        }

        use std::num::NonZeroI8;
        let nz = |n: i8| FontShift(NonZeroI8::new(n).expect("nonzero"));
        let attrs = [
            ForwardAttr::Bold,
            ForwardAttr::Gothic,
            ForwardAttr::Italic,
            ForwardAttr::SuperScript,
            ForwardAttr::SubScript,
            ForwardAttr::SmallScript(BoutenPosition::Right),
            ForwardAttr::Framed(EnclosureKind::Rule),
            ForwardAttr::Horizontal,
            ForwardAttr::Caption,
            ForwardAttr::FontSize(nz(2)),
            ForwardAttr::FontSize(nz(-1)),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
            ForwardAttr::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            },
            ForwardAttr::CombineUpright,
            ForwardAttr::Fraction,
            ForwardAttr::AccentDot,
            ForwardAttr::Accent(AccentMark::Acute),
            ForwardAttr::AlignEnd { offset: 3 },
        ];
        for attr in attrs {
            cover_forward_attr(attr);
            let mut store = NodeStore::new();
            let text_id = store.intern("X");
            let target = store.push_contents(&[Content::Plain(text_id)]);
            let f = ForwardFormat {
                attr,
                target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::None,
            };
            let inline = format_inline(f, &store, 0);
            // Every attribute resolves its target text; none returns an
            // empty-class placeholder span.
            let json = serde_json::to_string(&inline).expect("serialise inline");
            assert!(
                json.contains('X'),
                "{attr:?} must project the real target text: {json}"
            );
            assert!(
                !json.contains("ForwardFormat"),
                "{attr:?} must not emit a ForwardFormat Debug dump: {json}"
            );
        }
        let mut store = NodeStore::new();
        cover_segment(Segment::Text(store.intern("x")));
    }

    #[test]
    fn forward_payload_variants_project_semantic_content() {
        fn cover(payload: ForwardPayload) {
            match payload {
                ForwardPayload::None
                | ForwardPayload::NestedSource
                | ForwardPayload::AccentBody(_)
                | ForwardPayload::QuotedTarget(_) => {}
            }
        }

        let mut store = NodeStore::new();
        let plain = store.intern("X");
        let plain_target = store.push_contents(&[Content::Plain(plain)]);
        let none = ForwardPayload::None;
        cover(none);
        assert_eq!(
            format_inline(
                ForwardFormat {
                    attr: ForwardAttr::Bold,
                    target: plain_target,
                    origin: ForwardOrigin::SelfContained,
                    payload: none,
                },
                &store,
                0,
            ),
            Inline::Strong(vec![Inline::Str("X".to_owned())])
        );

        let run = store.intern("Sam");
        let body = store.intern("mは上ドット付き");
        let accent_target = store.push_contents(&[Content::Plain(run)]);
        let accent = ForwardPayload::AccentBody(body);
        cover(accent);
        let Inline::Span(_, composed) = format_inline(
            ForwardFormat {
                attr: ForwardAttr::AccentDot,
                target: accent_target,
                origin: ForwardOrigin::SelfContained,
                payload: accent,
            },
            &store,
            0,
        ) else {
            panic!("accent dot must be a Span")
        };
        assert_eq!(composed, vec![Inline::Str("Saṁ".to_owned())]);

        let nested_text = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let nested = store.intern(nested_text);
        let nested_target = store.push_contents(&[Content::Plain(nested)]);
        cover(ForwardPayload::NestedSource);
        let inline = format_inline(
            ForwardFormat {
                attr: ForwardAttr::Bouten {
                    kind: BoutenKind::Goma,
                    position: BoutenPosition::Right,
                },
                target: nested_target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::NestedSource,
            },
            &store,
            0,
        );
        let json = serde_json::to_string(&inline).expect("serialize nested projection");
        assert!(json.contains("aozora-gaiji"), "{json}");
        assert!(json.contains('枘'), "{json}");
        assert!(!json.contains("Char("), "{json}");
    }

    #[test]
    fn multi_character_accent_target_falls_back_verbatim() {
        let mut store = NodeStore::new();
        let text = store.intern("ex");
        let target = store.push_contents(&[Content::Plain(text)]);
        let Inline::Span(_, content) = format_inline(
            ForwardFormat {
                attr: ForwardAttr::Accent(AccentMark::Acute),
                target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::None,
            },
            &store,
            0,
        ) else {
            panic!("accent must be a Span")
        };
        assert_eq!(content, vec![Inline::Str("ex".to_owned())]);
    }

    #[test]
    fn nested_source_depth_limit_falls_back_to_verbatim_source() {
        let mut store = NodeStore::new();
        let raw = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let text = store.intern(raw);
        let target = store.push_contents(&[Content::Plain(text)]);
        let Inline::Span(_, content) = format_inline(
            ForwardFormat {
                attr: ForwardAttr::Bouten {
                    kind: BoutenKind::Goma,
                    position: BoutenPosition::Right,
                },
                target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::NestedSource,
            },
            &store,
            MAX_NESTED_SOURCE_DEPTH,
        ) else {
            panic!("bouten must be a Span")
        };
        assert_eq!(content, vec![Inline::Str(raw.to_owned())]);
    }

    #[test]
    fn block_nested_source_falls_back_to_verbatim_source() {
        let mut store = NodeStore::new();
        let raw = "［＃改ページ］";
        let text = store.intern(raw);
        let target = store.push_contents(&[Content::Plain(text)]);
        let inline = format_inline(
            ForwardFormat {
                attr: ForwardAttr::Bold,
                target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::NestedSource,
            },
            &store,
            0,
        );

        assert_eq!(inline, Inline::Strong(vec![Inline::Str(raw.to_owned())]));
    }

    #[test]
    fn nested_source_preserves_each_internal_newline() {
        let mut store = NodeStore::new();
        let text = store.intern("a\n\nb");
        let target = store.push_contents(&[Content::Plain(text)]);
        let inline = format_inline(
            ForwardFormat {
                attr: ForwardAttr::Bold,
                target,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::NestedSource,
            },
            &store,
            0,
        );

        assert_eq!(
            inline,
            Inline::Strong(vec![
                Inline::Str("a".to_owned()),
                Inline::LineBreak,
                Inline::LineBreak,
                Inline::Str("b".to_owned()),
            ])
        );
    }

    // -----------------------------------------------------------------
    // Block-leaf nodes
    // -----------------------------------------------------------------

    #[test]
    fn section_break_emits_classed_div() {
        let blocks = project("前章。\n［＃改丁］\n次章。\n");
        let (attr, inner) = find_div(&blocks, "section-break").expect("section-break div");
        assert!(
            attr.1.iter().any(|c| c.contains("section-break-")),
            "section-break carries a kind-specific class: {:?}",
            attr.1
        );
        assert!(inner.is_empty(), "section-break div is empty: {inner:?}");
    }

    #[test]
    fn page_break_closes_paragraph_and_emits_rule() {
        let blocks = project("第一章\n本文。\n［＃改ページ］\n第二章\n");
        let rule_idx = blocks
            .iter()
            .position(|b| matches!(b, Block::HorizontalRule))
            .expect("HorizontalRule present");
        assert!(
            matches!(&blocks[rule_idx - 1], Block::Para(_)),
            "page break flushes the preceding paragraph"
        );
    }

    #[test]
    fn large_heading_projects_to_header_level_1() {
        let blocks = project("第一章\n［＃「第一章」は大見出し］\n本文。\n");
        let header = blocks
            .iter()
            .find_map(|b| match b {
                Block::Header(level, attr, inlines) => Some((*level, attr, inlines)),
                _ => None,
            })
            .expect("Header block");
        assert_eq!(header.0, 1, "大見出し → level 1");
        assert_eq!(kv(header.1, "kind"), Some("large"), "large heading kind");
        assert!(
            kv(header.1, "style").is_none(),
            "standard style adds no style kv"
        );
    }

    #[test]
    fn window_heading_projects_with_style_attr() {
        let blocks = project("序章\n［＃「序章」は窓中見出し］\n");
        let header = blocks
            .iter()
            .find_map(|b| match b {
                Block::Header(level, attr, _) => Some((*level, attr)),
                _ => None,
            })
            .expect("Header block");
        assert_eq!(header.0, 2, "中見出し → level 2");
        assert_eq!(kv(header.1, "kind"), Some("medium"), "medium kind");
        assert_eq!(kv(header.1, "style"), Some("window"), "window style attr");
    }

    #[test]
    fn sashie_keyword_form_no_caption_has_empty_alt() {
        let blocks = project("ある日、［＃挿絵（cover.png）入る］その地に至る。\n");
        let img = find_image(&blocks).expect("sashie image");
        assert!(img.1.is_empty(), "no caption → empty alt: {:?}", img.1);
        assert_eq!(img.2.0, "cover.png", "image target file");
    }

    #[test]
    fn sashie_keyword_form_with_caption_emits_figure_caption() {
        let blocks = project("ある日、［＃挿絵（cover.png）「図一」入る］その地に至る。\n");
        let img = find_image(&blocks).expect("sashie image");
        assert_eq!(
            img.1,
            &[Inline::Str("図一".to_owned())],
            "caption becomes alt text"
        );
        let figure = blocks.iter().find_map(|block| match block {
            Block::Figure(attr, caption, _) => Some((attr, caption)),
            _ => None,
        });
        let (attr, (short, caption)) = figure.expect("semantic Figure block");
        assert!(has_class(attr, "illustration"));
        assert!(short.is_none());
        assert_eq!(
            caption,
            &vec![Block::Plain(vec![Inline::Str("図一".to_owned())])]
        );
    }

    #[test]
    fn sashie_caption_keeps_structured_gaiji() {
        let blocks =
            project("［＃「※［＃ローマ数字1、1-13-21］」のキャプション付きの図（fig.png）入る］");
        let (_, caption) = blocks
            .iter()
            .find_map(|block| match block {
                Block::Figure(attr, (_, caption), _) => Some((attr, caption)),
                _ => None,
            })
            .expect("Figure block");
        let [Block::Plain(inlines)] = caption.as_slice() else {
            panic!("expected structured gaiji caption, got {caption:?}");
        };
        let [Inline::Span(attr, content)] = inlines.as_slice() else {
            panic!("expected one gaiji inline, got {inlines:?}");
        };
        assert!(has_class(attr, "gaiji"));
        assert_eq!(content, &[Inline::Str("Ⅰ".to_owned())]);
    }

    #[test]
    fn sashie_trailing_plain_caption_reparses_nested_gaiji() {
        let blocks = project("［＃挿絵（cover.png）「※［＃「々」］」入る］");
        let (_, caption) = blocks
            .iter()
            .find_map(|block| match block {
                Block::Figure(attr, (_, caption), _) => Some((attr, caption)),
                _ => None,
            })
            .expect("Figure block");
        let [Block::Plain(inlines)] = caption.as_slice() else {
            panic!("expected structured gaiji caption: {caption:?}")
        };
        let [Inline::Span(attr, content)] = inlines.as_slice() else {
            panic!("expected structured gaiji caption: {inlines:?}")
        };
        assert!(has_class(attr, "gaiji"));
        assert_eq!(content, &[Inline::Str("々".to_owned())]);

        let (_, alt, _) = find_image(&blocks).expect("captioned image");
        assert!(matches!(alt, [Inline::Span(attr, _)] if has_class(attr, "gaiji")));
    }

    #[test]
    fn sashie_number_and_dimensions_are_structured_attributes() {
        let blocks = project("［＃挿絵1（fig.png、横100×縦200）「図一」入る］\n");
        let (figure_attr, _) = blocks
            .iter()
            .find_map(|block| match block {
                Block::Figure(attr, (_, caption), _) => Some((attr, caption)),
                _ => None,
            })
            .expect("Figure block");
        assert_eq!(kv(figure_attr, "number"), Some("1"));
        assert_eq!(kv(figure_attr, "dimensions"), Some("横100×縦200"));
        let (image_attr, _, _) = find_image(&blocks).expect("figure image");
        assert_eq!(kv(image_attr, "number"), Some("1"));
        assert_eq!(kv(image_attr, "dimensions"), Some("横100×縦200"));
    }

    #[test]
    fn sashie_general_form_uses_description_alt() {
        let blocks = project("［＃キャラクターの図（fig.png）入る］\n");
        let img = find_image(&blocks).expect("sashie image");
        assert_eq!(
            img.1,
            &[Inline::Str("キャラクターの図".to_owned())],
            "leading description becomes alt text"
        );
        assert_eq!(img.2.0, "fig.png", "image target file");
    }

    fn find_image(blocks: &[Block]) -> Option<(&Attr, &[Inline], &(String, String))> {
        fn find_inlines(inline_list: &[Inline]) -> Option<(&Attr, &[Inline], &(String, String))> {
            inline_list.iter().find_map(|inline| match inline {
                Inline::Image(attr, alt, target) => Some((attr, alt.as_slice(), target)),
                Inline::Span(_, nested)
                | Inline::Emph(nested)
                | Inline::Strong(nested)
                | Inline::Superscript(nested)
                | Inline::Subscript(nested) => find_inlines(nested),
                _ => None,
            })
        }
        blocks.iter().find_map(|block| match block {
            Block::Plain(content) | Block::Para(content) | Block::Header(_, _, content) => {
                find_inlines(content)
            }
            Block::Div(_, nested) => find_image(nested),
            Block::Figure(_, (_, caption), nested) => {
                find_image(caption).or_else(|| find_image(nested))
            }
            _ => None,
        })
    }

    // -----------------------------------------------------------------
    // Container attr arms
    // -----------------------------------------------------------------

    #[test]
    fn indent_container_carries_amount() {
        let blocks =
            project("本文。\n［＃ここから2字下げ］\n中身。\n［＃ここで字下げ終わり］\n後。\n");
        let (attr, _) = find_div(&blocks, "container-indent").expect("indent div");
        assert_eq!(kv(attr, "amount"), Some("2"), "indent amount");
        assert!(kv(attr, "wrap").is_none(), "plain indent has no wrap kv");
        assert!(
            kv(attr, "center").is_none(),
            "plain indent has no center kv"
        );
    }

    #[test]
    fn wrap_indent_container_carries_wrap_kv() {
        let blocks = project(
            "［＃ここから２字下げ、折り返して４字下げ］\n本文。\n［＃ここで字下げ終わり］\n",
        );
        let (attr, _) = find_div(&blocks, "container-indent").expect("indent div");
        assert_eq!(kv(attr, "amount"), Some("2"), "indent base amount");
        assert_eq!(kv(attr, "wrap"), Some("4"), "hanging-indent wrap amount");
    }

    #[test]
    fn center_indent_container_carries_center_kv() {
        let blocks =
            project("［＃ここから5字下げ、ページの左右中央に］\n献辞\n［＃ここで字下げ終わり］\n");
        let (attr, _) = find_div(&blocks, "container-indent").expect("indent div");
        assert_eq!(kv(attr, "amount"), Some("5"), "indent amount");
        assert_eq!(kv(attr, "center"), Some("true"), "page-centred indent kv");
    }

    #[test]
    fn warichu_block_container_div() {
        let blocks =
            project("前文。\n［＃ここから割り注］\n上／下\n［＃ここで割り注終わり］\n後文。\n");
        let div = find_div(&blocks, "container-warichu");
        assert!(div.is_some(), "warichu container div: {blocks:?}");
    }

    #[test]
    fn keigakomi_container_div() {
        let blocks = project("［＃罫囲み］\n本文一行目。\n本文二行目。\n［＃罫囲み終わり］\n");
        let (_, inner) = find_div(&blocks, "container-keigakomi").expect("keigakomi div");
        assert!(!inner.is_empty(), "keigakomi wraps inner blocks");
    }

    #[test]
    fn align_end_container_carries_offset() {
        let blocks = project("［＃ここから地から3字上げ］\n名簿。\n［＃ここで字上げ終わり］\n");
        let (attr, _) = find_div(&blocks, "container-align-end").expect("align-end div");
        assert_eq!(kv(attr, "offset"), Some("3"), "align-end offset");
    }

    // Post-S5, a *text-only* bouten range folds to an inline forward span (see
    // `bouten_carries_kind_and_position`), so the `container-bouten` div path is
    // reached only by a range whose run is non-foldable. Embedded ruby keeps the
    // range a container while leaving the open marker's variant / position intact.
    #[test]
    fn bouten_range_container_carries_variant() {
        let blocks = project("本文［＃傍点］甲《こう》［＃傍点終わり］。");
        let (attr, _) = find_span(&blocks, "container-bouten").expect("bouten range span");
        assert_eq!(kv(attr, "variant"), Some("goma"), "default bouten variant");
        assert_eq!(kv(attr, "position"), Some("right"));
    }

    #[test]
    fn bouten_range_left_position_kv() {
        let blocks = project("本文［＃左に傍線］丙《へい》［＃左に傍線終わり］。");
        let (attr, _) = find_span(&blocks, "container-bouten").expect("bouten range span");
        assert_eq!(kv(attr, "variant"), Some("bosen"), "傍線 variant slug");
        assert_eq!(kv(attr, "position"), Some("left"), "left-side range kv");
    }

    #[test]
    fn columns_container_carries_count() {
        let blocks =
            project("前文。\n［＃ここから2段組み］\n左右。\n［＃ここで段組み終わり］\n後文。\n");
        let (attr, _) = find_div(&blocks, "container-columns").expect("columns div");
        assert_eq!(kv(attr, "count"), Some("2"));
    }

    #[test]
    fn table_container_has_semantic_class() {
        let blocks = project("［＃ここから表］\n項目\u{3000}値\n［＃ここで表終わり］\n");
        assert!(find_div(&blocks, "container-table").is_some(), "{blocks:?}");
    }

    #[test]
    fn bold_block_container_carries_padding() {
        let blocks = project("［＃ここから太字］\n強調する段落。\n［＃ここで太字終わり］\n");
        let (attr, _) = find_div(&blocks, "container-bold").expect("bold div");
        assert_eq!(kv(attr, "padded"), Some("true"));
    }

    #[test]
    fn inline_region_remains_inside_one_paragraph() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Bold { padded: false });
        converter.push_inline(Inline::Str("inside".to_owned()));
        converter.close_container(true);
        converter.run();

        let [Block::Para(inlines)] = converter.blocks.as_slice() else {
            panic!("inline region must remain a Para: {:?}", converter.blocks)
        };
        assert!(matches!(
            inlines.as_slice(),
            [Inline::Span(attr, content)]
                if has_class(attr, "container-bold")
                    && matches!(content.as_slice(), [Inline::Strong(_)])
        ));
    }

    #[test]
    fn gothic_region_is_a_typeface_span_not_bold() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Gothic { padded: false });
        converter.push_inline(Inline::Str("gothic".to_owned()));
        converter.close_container(true);
        converter.run();

        let (_, content) = find_span(&converter.blocks, "container-gothic").expect("gothic span");
        assert_eq!(content, &[Inline::Str("gothic".to_owned())]);
        assert!(
            !content
                .iter()
                .any(|inline| matches!(inline, Inline::Strong(_)))
        );
    }

    #[test]
    fn nested_inline_regions_preserve_nesting_order() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Bold { padded: false });
        converter.push_inline(Inline::Str("outer-before".to_owned()));
        converter.open_container(RegionFormat::Italic { padded: false });
        converter.push_inline(Inline::Str("inner".to_owned()));
        converter.close_container(true);
        converter.push_inline(Inline::Str("outer-after".to_owned()));
        converter.close_container(true);
        converter.run();

        let (_, outer) = find_span(&converter.blocks, "container-bold").expect("outer span");
        let [Inline::Strong(outer)] = outer else {
            panic!("bold native wrapper missing: {outer:?}")
        };
        assert_eq!(outer.first(), Some(&Inline::Str("outer-before".to_owned())));
        assert!(matches!(
            outer.get(1),
            Some(Inline::Span(attr, _)) if has_class(attr, "container-italic")
        ));
        assert_eq!(outer.get(2), Some(&Inline::Str("outer-after".to_owned())));
    }

    #[test]
    fn inline_region_reopens_across_paragraph_boundaries() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Italic { padded: false });
        converter.push_inline(Inline::Str("first".to_owned()));
        converter.flush_paragraph();
        converter.push_inline(Inline::Str("second".to_owned()));
        converter.close_container(true);
        converter.run();

        assert_eq!(converter.blocks.len(), 2, "{:?}", converter.blocks);
        for block in &converter.blocks {
            let Block::Para(inlines) = block else {
                panic!("expected paragraph: {block:?}")
            };
            assert!(matches!(
                inlines.as_slice(),
                [Inline::Span(attr, _)] if has_class(attr, "container-italic")
            ));
        }
    }

    #[test]
    fn empty_inline_region_emits_exactly_one_empty_span() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Caption { padded: false });
        converter.close_container(true);
        converter.run();

        let [Block::Para(inlines)] = converter.blocks.as_slice() else {
            panic!("empty inline marker belongs to one paragraph")
        };
        assert!(matches!(
            inlines.as_slice(),
            [Inline::Span(attr, content)]
                if has_class(attr, "container-caption") && content.is_empty()
        ));
    }

    #[test]
    fn block_nested_in_inline_region_preserves_source_order() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Bold { padded: false });
        converter.push_inline(Inline::Str("before".to_owned()));
        converter.open_container(RegionFormat::Indent(IndentBlock {
            amount: 2,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        }));
        converter.push_inline(Inline::Str("block".to_owned()));
        converter.close_container(false);
        converter.push_inline(Inline::Str("after".to_owned()));
        converter.close_container(true);
        converter.run();

        assert_eq!(converter.blocks.len(), 3, "{:?}", converter.blocks);
        assert!(matches!(&converter.blocks[0], Block::Para(_)));
        assert!(matches!(
            &converter.blocks[1],
            Block::Div(attr, _) if has_class(attr, "container-indent")
        ));
        assert!(matches!(&converter.blocks[2], Block::Para(_)));
        let json = serde_json::to_string(&converter.blocks).expect("serialize blocks");
        let before = json.find("before").expect("before text");
        let block = json.find("block").expect("block text");
        let after = json.find("after").expect("after text");
        assert!(before < block && block < after, "{json}");
    }

    #[test]
    fn heading_region_with_phrasing_content_becomes_header() {
        let store = NodeStore::new();
        let mut converter = Converter::new("", &[], &store, 0);
        converter.open_container(RegionFormat::Heading {
            level: HeadingKind::Medium,
            style: HeadingStyle::Window,
            padded: true,
        });
        converter.push_inline(Inline::Str("heading".to_owned()));
        converter.close_container(false);
        converter.run();

        let [Block::Header(level, attr, content)] = converter.blocks.as_slice() else {
            panic!(
                "heading region should become Header: {:?}",
                converter.blocks
            )
        };
        assert_eq!(*level, 2);
        assert_eq!(kv(attr, "style"), Some("window"));
        assert_eq!(kv(attr, "padded"), Some("true"));
        assert_eq!(content, &[Inline::Str("heading".to_owned())]);
    }

    #[test]
    fn nested_containers_nest_divs() {
        let blocks = project(
            "［＃ここから2字下げ］\n外。\n［＃ここから3字下げ］\n内。\n\
             ［＃ここで字下げ終わり］\n戻る。\n［＃ここで字下げ終わり］\n",
        );
        let (_, outer) = find_div(&blocks, "container-indent").expect("outer indent div");
        let has_nested = find_div(outer, "container-indent").is_some();
        assert!(has_nested, "inner indent div nested in outer: {outer:?}");
    }

    // -----------------------------------------------------------------
    // Defensive stack handling
    // -----------------------------------------------------------------

    #[test]
    fn unclosed_container_is_popped_at_eof() {
        // No matching close — `run` must still wrap the body in a Div.
        let blocks = project("［＃ここから2字下げ］\n本文。\n");
        assert!(
            find_div(&blocks, "container-indent").is_some(),
            "unclosed container still wraps its body: {blocks:?}"
        );
    }

    #[test]
    fn unmatched_close_does_not_panic_and_keeps_body() {
        // A close with no matching open must not pop the root frame.
        let blocks = project("本文。\n［＃ここで字下げ終わり］\n");
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Para(_))),
            "body survives an unmatched close: {blocks:?}"
        );
        assert!(
            find_div(&blocks, "container-indent").is_none(),
            "no spurious container div: {blocks:?}"
        );
    }

    #[test]
    fn empty_source_yields_no_blocks() {
        let blocks = project("");
        assert!(blocks.is_empty(), "empty source → no blocks: {blocks:?}");
    }

    #[test]
    fn pandoc_api_version_is_pinned() {
        let doc = Document::new("x");
        let pandoc = to_pandoc(&doc.snapshot());
        assert_eq!(
            pandoc.pandoc_api_version,
            vec![1, 23],
            "pinned Pandoc 1.23 API version"
        );
        assert!(pandoc.meta.is_empty(), "no meta emitted");
    }

    // -----------------------------------------------------------------
    // Direct builder unit tests (forms not reachable as inline leaves
    // through the source pipeline, but live projection helpers)
    // -----------------------------------------------------------------

    #[test]
    fn line_inline_indent_carries_amount() {
        let inline = line_inline(LineFormat::Indent {
            amount: 4,
            end_offset: None,
        });
        match inline {
            Inline::Span(attr, inner) => {
                assert!(has_class(&attr, "indent"), "indent class: {:?}", attr.1);
                assert_eq!(kv(&attr, "amount"), Some("4"), "indent amount kv");
                assert!(inner.is_empty(), "indent is an empty marker span");
            }
            other => panic!("expected Span, got {other:?}"),
        }
    }

    #[test]
    fn line_inline_align_end_carries_offset() {
        let inline = line_inline(LineFormat::AlignEnd { offset: 7 });
        match inline {
            Inline::Span(attr, _) => {
                assert!(has_class(&attr, "align-end"), "align-end class");
                assert_eq!(kv(&attr, "offset"), Some("7"), "align-end offset kv");
            }
            other => panic!("expected Span, got {other:?}"),
        }
    }

    #[test]
    fn line_inline_center_is_empty_marker() {
        match line_inline(LineFormat::Center { page: false }) {
            Inline::Span(attr, inner) => {
                assert!(has_class(&attr, "center"), "center class");
                assert!(inner.is_empty(), "center marker span is empty");
            }
            other => panic!("expected Span, got {other:?}"),
        }
    }

    #[test]
    fn bouten_position_slug_covers_left_and_unknown() {
        assert_eq!(bouten_position_slug(BoutenPosition::Right), "right");
        assert_eq!(bouten_position_slug(BoutenPosition::Left), "left");
        // The `Both` arm is distinct from the `_ => "unknown"` fallthrough:
        // deleting it must not collapse 両側 into `unknown`.
        assert_eq!(bouten_position_slug(BoutenPosition::Both), "both");
    }

    #[test]
    fn annotation_kind_slug_covers_all_named_arms() {
        for (kind, slug) in [
            (DirectiveKind::NonCanonical, "non-canonical"),
            (DirectiveKind::Editorial, "editorial"),
            (DirectiveKind::Sic, "sic"),
            (DirectiveKind::BaseTextVariant, "base-text-variant"),
            (DirectiveKind::WarichuOpen, "warichu-open"),
            (DirectiveKind::WarichuClose, "warichu-close"),
            (DirectiveKind::Empty, "empty"),
            (DirectiveKind::EditorNote, "editor-note"),
            (DirectiveKind::RubyAttached, "ruby-attached"),
            (DirectiveKind::RubyRetarget, "ruby-retarget"),
            (DirectiveKind::RubyPairOpen, "ruby-pair-open"),
            (DirectiveKind::RubyPairClose, "ruby-pair-close"),
            (DirectiveKind::MarginNotePairOpen, "margin-note-pair-open"),
            (DirectiveKind::MarginNotePairClose, "margin-note-pair-close"),
        ] {
            assert_eq!(
                annotation_kind_slug(kind),
                slug,
                "{kind:?} must map to its own slug, not the `other` fallthrough"
            );
        }
    }

    #[test]
    fn heading_kind_slug_covers_levels() {
        assert_eq!(heading_kind_slug(HeadingKind::Large), "large");
        assert_eq!(heading_kind_slug(HeadingKind::Medium), "medium");
        assert_eq!(heading_kind_slug(HeadingKind::Small), "small");
    }

    #[test]
    fn heading_style_slug_covers_styles() {
        assert_eq!(
            heading_style_slug(HeadingStyle::SameLine),
            Some("same-line")
        );
        assert_eq!(heading_style_slug(HeadingStyle::Window), Some("window"));
        assert_eq!(
            heading_style_slug(HeadingStyle::Standard),
            None,
            "standard style adds no slug"
        );
    }

    #[test]
    fn small_heading_block_builder_is_level_3() {
        // Build the owned heading payload directly via a store.
        let mut store = NodeStore::new();
        let text_id = store.intern("見出し");
        let text = store.push_contents(&[Content::Plain(text_id)]);
        let heading = Heading {
            kind: HeadingKind::Small,
            style: HeadingStyle::SameLine,
            text,
        };
        match aozora_heading_block(heading, &store, 0) {
            Block::Header(level, attr, inlines) => {
                assert_eq!(level, 3, "小見出し → level 3");
                assert_eq!(kv(&attr, "kind"), Some("small"), "small kind slug");
                assert_eq!(
                    kv(&attr, "style"),
                    Some("same-line"),
                    "same-line style attr"
                );
                assert_eq!(
                    inlines,
                    vec![Inline::Str("見出し".to_owned())],
                    "heading text inlines"
                );
            }
            other => panic!("expected Header, got {other:?}"),
        }
    }

    /// A container close emits its `Div` mid-stream, so content that follows
    /// the close is a *sibling* block at the enclosing level — not swallowed
    /// into the container. A no-op `close_container` would keep the frame open
    /// until EOF, folding the trailing content inside the Div.
    #[test]
    fn close_container_emits_div_before_trailing_content() {
        let blocks = project("A\n\n［＃ここから2字下げ］\nB\n［＃ここで字下げ終わり］\n\nC\n");
        let div_idx = blocks
            .iter()
            .position(|b| matches!(b, Block::Div(a, _) if has_class(a, "container-indent")))
            .expect("indent div present");
        let trailing_is_sibling = blocks[div_idx + 1..].iter().any(|b| {
            matches!(
                b,
                Block::Para(inlines)
                    if inlines.iter().any(|i| matches!(i, Inline::Str(s) if s == "C"))
            )
        });
        assert!(
            trailing_is_sibling,
            "trailing content must be a sibling Para after the closed Div: {blocks:?}"
        );
    }

    /// `push_content_inlines` dispatches a `Content::Segments` run element by
    /// element: `Text` → `Str`, `Gaiji` → gaiji span, `Directive` → annotation
    /// span. Deleting the `Segments` arm drops the whole run; deleting any
    /// segment arm swaps that element for an empty placeholder `Str`.
    #[test]
    fn push_content_inlines_dispatches_each_segment_kind() {
        use crate::syntax::ast::GaijiCanonicalOwned;

        let mut store = NodeStore::new();
        let text_id = store.intern("字");
        let gaiji_hint = store.intern("外字の説明");
        let raw_id = store.intern("ママ");
        let gaiji = Gaiji {
            hint: gaiji_hint,
            canonical: GaijiCanonicalOwned::Unicode('A'),
            mencode_separator: true,
            standalone: false,
        };
        let directive = Directive {
            raw: raw_id,
            kind: DirectiveKind::Sic,
        };
        let seg = store.push_segments(&[
            Segment::Text(text_id),
            Segment::Gaiji(gaiji),
            Segment::Directive(directive),
        ]);
        let mut buf = Vec::new();
        push_content_inlines(Content::Segments(seg), &store, 0, &mut buf);

        assert_eq!(buf.len(), 3, "one inline per segment: {buf:?}");
        assert_eq!(
            buf[0],
            Inline::Str("字".to_owned()),
            "Text segment projects to its interned Str"
        );
        match &buf[1] {
            Inline::Span(attr, _) => assert!(
                has_class(attr, "gaiji"),
                "Gaiji segment projects to a gaiji span: {attr:?}"
            ),
            other => panic!("expected gaiji Span, got {other:?}"),
        }
        match &buf[2] {
            Inline::Span(attr, _) => {
                assert!(
                    has_class(attr, "annotation"),
                    "Directive segment projects to an annotation span: {attr:?}"
                );
                assert_eq!(
                    kv(attr, "kind"),
                    Some("sic"),
                    "the directive's kind slug rides through"
                );
            }
            other => panic!("expected annotation Span, got {other:?}"),
        }
    }

    /// A `container-bouten` range with the `両側` (both-side) position emits a
    /// `position=both` kv. Deleting the `Both` arm drops the kv entirely.
    #[test]
    fn container_attr_bouten_both_position_kv() {
        let attr = container_attr(RegionFormat::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Both,
        });
        assert_eq!(
            kv(&attr, "variant"),
            Some("goma"),
            "goma bouten variant slug"
        );
        assert_eq!(
            kv(&attr, "position"),
            Some("both"),
            "both-side range emits the position=both kv"
        );
    }

    /// Co-applied indent styles (#78) join into a space-separated `modifiers`
    /// kv; a style-free indent emits none. Pins both sides of the
    /// `if !modifiers.is_empty()` guard.
    #[test]
    fn container_attr_indent_modifiers_kv() {
        let styled = container_attr(RegionFormat::Indent(IndentBlock {
            amount: 3,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles {
                gothic: true,
                horizontal: true,
                framed: false,
                font: None,
            },
        }));
        assert_eq!(
            kv(&styled, "modifiers"),
            Some("gothic horizontal"),
            "co-applied styles join into a modifiers kv"
        );
        let plain = container_attr(RegionFormat::Indent(IndentBlock {
            amount: 3,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        }));
        assert!(
            kv(&plain, "modifiers").is_none(),
            "a style-free indent emits no modifiers kv"
        );
    }

    #[test]
    fn every_region_format_has_a_named_projection() {
        for region in RegionFormat::ALL {
            let attr = container_attr(region);
            assert_eq!(attr.1.len(), 1, "{region:?}: {attr:?}");
            assert!(attr.1[0].starts_with(AOZORA_CLASS_PREFIX), "{region:?}");
            assert!(!attr.1[0].contains("unknown"), "{region:?}: {attr:?}");
        }
    }
}
