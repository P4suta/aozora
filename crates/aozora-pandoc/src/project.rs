//! Source-driven projection from an [`OwnedLexOutput`] to a
//! [`pandoc_ast::Pandoc`] document.
//!
//! Walks the source linearly, slicing it into spans by the owned
//! `source_nodes` side-table. Plain runs flow into Pandoc inlines
//! verbatim (with `\n\n` paragraph splits and single `\n` →
//! `SoftBreak`). Each classified node lifts to a Pandoc inline /
//! block construct as documented in [`crate`]. Payload text is resolved
//! through the output's [`NodeStore`].

use aozora::{
    BoutenPosition, DirectiveKind, Format, ForwardAttr, HeadingKind, HeadingStyle, IndentBlock,
    IndentLayout, LineFormat, RegionFormat, SectionKind, Span, roman_slug,
    syntax::{
        borrowed::ForwardOrigin,
        owned::{
            AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
            GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned, KaeritenOwned,
            MarginNoteOwned, NodeOwned, NodeRefOwned, NodeStore, OwnedLexOutput, RubyOwned,
            SegmentOwned, SourceNodeOwned, WarichuOwned,
        },
    },
};
use pandoc_ast::{Attr, Block, Inline, Pandoc};

use crate::AOZORA_CLASS_PREFIX;

/// Lift a parsed [`OwnedLexOutput`] to a [`pandoc_ast::Pandoc`] document.
///
/// See the crate-level docs for the projection rules.
#[must_use]
pub fn to_pandoc(out: &OwnedLexOutput) -> Pandoc {
    // `source_nodes` index into the sanitize-stage buffer, not the raw
    // user-supplied source. The owned lex output carries exactly that buffer
    // in `sanitized`, so the slice base already matches the source-node
    // coordinate system — no re-sanitize is needed (the borrowed projection
    // had to re-run sanitize to reconstruct it).
    let mut converter = Converter::new(&out.sanitized, &out.source_nodes, &out.store);
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
    container: Option<RegionFormat>,
}

impl Frame {
    fn root() -> Self {
        Self {
            blocks: Vec::new(),
            inlines: None,
            container: None,
        }
    }

    fn child(kind: RegionFormat) -> Self {
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
    nodes: &'src [SourceNodeOwned],
    /// Backing store the owned nodes' `StrId` / range payloads resolve against.
    store: &'src NodeStore,
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
    fn new(source: &'src str, nodes: &'src [SourceNodeOwned], store: &'src NodeStore) -> Self {
        Self {
            source,
            nodes,
            store,
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

    fn dispatch_node(&mut self, entry: &SourceNodeOwned) {
        match entry.node {
            NodeRefOwned::Inline(node) => self.dispatch_inline_node(node, entry.source_span),
            NodeRefOwned::BlockLeaf(node) => self.dispatch_block_leaf(node, entry.source_span),
            NodeRefOwned::BlockOpen(kind) => self.open_container(kind),
            NodeRefOwned::BlockClose(_) => self.close_container(),
            // `NodeRefOwned` is `#[non_exhaustive]`; treat unknown
            // variants as pass-through plain text.
            _ => {}
        }
    }

    fn dispatch_inline_node(&mut self, node: NodeOwned, _span: Span) {
        use NodeOwned as N;
        // A `Referenced` forward keeps its target literal in the upstream plain
        // run (or a ruby base); projecting `f.target` here would double it
        // (#231). Mirror the HTML renderer's origin gate (#228,
        // `aozora-render::render_node::render_format`): emit nothing for
        // `Referenced` so the literal is rendered once, upstream. Gated before
        // the `format_inline` attr match so the debug-span fallback can't leak
        // a duplicate either. The emphasis markup is dropped rather than
        // duplicated — a value-returning projection cannot retroactively wrap an
        // already-emitted run, same as the streaming HTML path.
        if let N::Format(f) = node
            && matches!(f.origin, ForwardOrigin::Referenced)
        {
            return;
        }
        let store = self.store;
        let inline = match node {
            N::Ruby(r) => ruby_inline(r, store),
            N::MarginNote(s) => side_note_inline(s, store),
            N::Format(f) => format_inline(f, store),
            N::Gaiji(g) => gaiji_inline(g, store),
            N::Line(lf) => line_inline(lf),
            N::Warichu(w) => warichu_inline(w, store),
            N::Directive(a) => annotation_inline(a, store),
            N::Kaeriten(k) => kaeriten_inline(k, store),
            N::AngleQuote(d) => angle_quote_inline(d, store),
            N::HeadingHint(h) => heading_hint_inline(h, store),
            // 改行 — an in-paragraph forced break (inline leaf).
            N::ForcedBreak => Inline::LineBreak,
            // Block-leaf variants slip through here only if the
            // pipeline misclassified them; render as fallback span.
            // The debug form is the owned node's `Debug` (a non-canonical
            // placeholder; substring-stable for the `*Owned` type names).
            other => Inline::Span(plain_attr(), vec![Inline::Str(format!("{other:?}"))]),
        };
        self.current_frame_mut().paragraph().push(inline);
    }

    fn dispatch_block_leaf(&mut self, node: NodeOwned, _span: Span) {
        use NodeOwned as N;
        // Block-leaf nodes close any in-flight paragraph and emit a
        // standalone block.
        self.current_frame_mut().flush_paragraph();
        let store = self.store;
        let block = match node {
            N::PageBreak => Block::HorizontalRule,
            // 本文終わり — a distinct structural marker Div (a colophon follows).
            N::BodyEnd => Block::Div(
                (
                    String::new(),
                    vec![format!("{AOZORA_CLASS_PREFIX}body-end")],
                    Vec::new(),
                ),
                Vec::new(),
            ),
            N::SectionBreak(k) => section_break_block(k),
            N::Heading(h) => aozora_heading_block(h, store),
            N::Illustration(s) => sashie_block(s, store),
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

    fn open_container(&mut self, kind: RegionFormat) {
        // A new container starts a new block context; flush any
        // in-flight paragraph in the current frame first.
        self.current_frame_mut().flush_paragraph();
        self.stack.push(Frame::child(kind));
    }

    fn close_container(&mut self) {
        // Adversarial / malformed input can emit a BlockClose without
        // a matching open (the lex pipeline emits a diagnostic but
        // still surfaces the close in `source_nodes`). Popping the
        // root frame would leave the converter with an empty stack
        // and panic on the next `current_frame_mut`. Bottom-of-stack
        // is the document root, so we keep at least one frame.
        if self.stack.len() <= 1 {
            return;
        }
        let frame = self.stack.pop().expect("len > 1 above ⇒ pop yields Some");
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

/// Resolve a [`ContentRange`] payload field (ruby base/reading, forward
/// target, …) to its Pandoc inlines.
fn content_range_to_inlines(range: ContentRange, store: &NodeStore) -> Vec<Inline> {
    let mut buf = Vec::new();
    for &content in store.resolve_content_range(range) {
        push_content_inlines(content, store, &mut buf);
    }
    buf
}

/// Resolve a bare [`ContentOwned`] payload field (warichu upper/lower,
/// sashie caption) to its Pandoc inlines.
fn content_to_inlines(content: ContentOwned, store: &NodeStore) -> Vec<Inline> {
    let mut buf = Vec::new();
    push_content_inlines(content, store, &mut buf);
    buf
}

fn push_content_inlines(content: ContentOwned, store: &NodeStore, buf: &mut Vec<Inline>) {
    match content {
        ContentOwned::Plain(id) => buf.push(Inline::Str(store.resolve_str(id).to_owned())),
        ContentOwned::Segments(range) => {
            for &seg in store.resolve_seg_range(range) {
                match seg {
                    SegmentOwned::Text(id) => {
                        buf.push(Inline::Str(store.resolve_str(id).to_owned()));
                    }
                    SegmentOwned::Gaiji(g) => buf.push(gaiji_inline(g, store)),
                    SegmentOwned::Directive(a) => buf.push(annotation_inline(a, store)),
                    // `SegmentOwned` is `#[non_exhaustive]`; future segment
                    // kinds get a placeholder until projection logic is added.
                    _ => buf.push(Inline::Str(String::new())),
                }
            }
        }
        // `ContentOwned` is `#[non_exhaustive]`.
        _ => {}
    }
}

fn ruby_inline(r: RubyOwned, store: &NodeStore) -> Inline {
    let base_inlines = content_range_to_inlines(r.base, store);
    let reading_inlines = content_range_to_inlines(r.reading, store);
    let inner = vec![
        Inline::Span(class_attr("ruby-base"), base_inlines),
        Inline::Span(class_attr("ruby-reading"), reading_inlines),
    ];
    Inline::Span(class_attr("ruby"), inner)
}

fn side_note_inline(s: MarginNoteOwned, store: &NodeStore) -> Inline {
    let base_inlines = content_range_to_inlines(s.base, store);
    let note_inlines = content_range_to_inlines(s.note, store);
    let inner = vec![
        Inline::Span(class_attr("sidenote-base"), base_inlines),
        Inline::Span(class_attr("sidenote-note"), note_inlines),
    ];
    Inline::Span(class_attr("sidenote"), inner)
}

/// Project a forward-reference emphasis node. 傍点 / 傍線 and 縦中横 carry
/// dedicated spans; every other attribute (太字 / 斜体 / 小書き / …) falls
/// through to a debug span, the legacy pandoc behaviour.
fn format_inline(f: ForwardFormatOwned, store: &NodeStore) -> Inline {
    match f.attr {
        ForwardAttr::Bouten { kind, position } => {
            let attr = class_attr_kv(
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
            );
            Inline::Span(attr, content_range_to_inlines(f.target, store))
        }
        ForwardAttr::CombineUpright => Inline::Span(
            class_attr("tate-chu-yoko"),
            content_range_to_inlines(f.target, store),
        ),
        // Debug placeholder for un-projected emphasis (font-size / superscript
        // / 太字 / 斜体 / …): the owned format's `Debug`, substring-stable on
        // `ForwardFormat` / the attr name but not byte-identical to the
        // borrowed dump.
        _ => Inline::Span(plain_attr(), vec![Inline::Str(format!("{f:?}"))]),
    }
}

fn bouten_position_slug(p: BoutenPosition) -> &'static str {
    match p {
        BoutenPosition::Right => "right",
        BoutenPosition::Left => "left",
        _ => "unknown",
    }
}

fn gaiji_inline(g: GaijiOwned, store: &NodeStore) -> Inline {
    let mut kvs = vec![(
        "description".to_owned(),
        store.resolve_str(g.hint).to_owned(),
    )];
    if g.canonical.has_mencode() {
        let mut mencode = String::new();
        g.canonical
            .write_mencode(store, &mut mencode)
            .expect("write_mencode into String is infallible");
        kvs.push(("mencode".to_owned(), mencode));
    }
    let inner = g.resolve(store).map_or_else(
        || vec![Inline::Str("〓".to_owned())],
        |resolved| vec![Inline::Str(format!("{resolved:?}"))],
    );
    Inline::Span(class_attr_kv("gaiji", kvs), inner)
}

/// Project a single-line layout directive (字下げ / 地付き / 中央 / 罫囲み).
fn line_inline(lf: LineFormat) -> Inline {
    let attr = match lf {
        LineFormat::Indent { amount } => {
            class_attr_kv("indent", vec![("amount".to_owned(), amount.to_string())])
        }
        LineFormat::AlignEnd { offset } => {
            class_attr_kv("align-end", vec![("offset".to_owned(), offset.to_string())])
        }
        LineFormat::Center { .. } => class_attr_kv("center", Vec::new()),
        LineFormat::Framed => class_attr("keigakomi"),
        _ => plain_attr(),
    };
    Inline::Span(attr, Vec::new())
}

fn warichu_inline(w: WarichuOwned, store: &NodeStore) -> Inline {
    let upper = Inline::Span(
        class_attr("warichu-upper"),
        content_to_inlines(w.upper, store),
    );
    let lower = Inline::Span(
        class_attr("warichu-lower"),
        content_to_inlines(w.lower, store),
    );
    Inline::Span(class_attr("warichu"), vec![upper, lower])
}

fn annotation_inline(a: DirectiveOwned, store: &NodeStore) -> Inline {
    Inline::Span(
        class_attr_kv(
            "annotation",
            vec![
                ("kind".to_owned(), annotation_kind_slug(a.kind).to_owned()),
                ("raw".to_owned(), store.resolve_str(a.raw).to_owned()),
            ],
        ),
        Vec::new(),
    )
}

fn annotation_kind_slug(k: DirectiveKind) -> &'static str {
    match k {
        DirectiveKind::Unknown => "unknown",
        DirectiveKind::Sic => "sic",
        DirectiveKind::BaseTextVariant => "base-text-variant",
        DirectiveKind::InvalidRubySpan => "invalid-ruby-span",
        _ => "other",
    }
}

fn kaeriten_inline(k: KaeritenOwned, store: &NodeStore) -> Inline {
    Inline::Span(
        class_attr_kv(
            "kaeriten",
            vec![("mark".to_owned(), store.resolve_str(k.mark).to_owned())],
        ),
        Vec::new(),
    )
}

fn angle_quote_inline(d: AngleQuoteOwned, store: &NodeStore) -> Inline {
    Inline::Span(
        class_attr("angle-quote"),
        content_range_to_inlines(d.content, store),
    )
}

fn heading_hint_inline(h: HeadingHintOwned, store: &NodeStore) -> Inline {
    Inline::Span(
        class_attr_kv(
            "heading-hint",
            vec![
                ("level".to_owned(), h.level.outline_level().to_string()),
                ("target".to_owned(), store.resolve_str(h.target).to_owned()),
            ],
        ),
        Vec::new(),
    )
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

fn aozora_heading_block(h: HeadingOwned, store: &NodeStore) -> Block {
    let level: i64 = match h.kind {
        HeadingKind::Large => 1,
        HeadingKind::Medium => 2,
        HeadingKind::Small => 3,
        _ => 4,
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
        content_range_to_inlines(h.text, store),
    )
}

fn heading_kind_slug(k: HeadingKind) -> &'static str {
    match k {
        HeadingKind::Large => "large",
        HeadingKind::Medium => "medium",
        HeadingKind::Small => "small",
        _ => "other",
    }
}

/// Style modifier slug, or `None` for the standard style (which adds no
/// `style` attribute, keeping a standard heading's projection unchanged).
fn heading_style_slug(s: HeadingStyle) -> Option<&'static str> {
    match s {
        HeadingStyle::SameLine => Some("same-line"),
        HeadingStyle::Window => Some("window"),
        // Standard (and any future `#[non_exhaustive]` style) adds no attr.
        _ => None,
    }
}

fn sashie_block(s: IllustrationOwned, store: &NodeStore) -> Block {
    // The general form's leading description is the alt; otherwise the
    // keyword 挿絵 form's trailing 「caption」 is the next-best alt text.
    let alt = s.description.map_or_else(
        || {
            s.caption
                .map(|c| content_to_inlines(c, store))
                .unwrap_or_default()
        },
        |description| vec![Inline::Str(store.resolve_str(description).to_owned())],
    );
    let target = (store.resolve_str(s.file).to_owned(), String::new());
    Block::Para(vec![Inline::Image(class_attr("sashie"), alt, target)])
}

fn container_attr(kind: RegionFormat) -> Attr {
    let (slug, kvs): (&str, Vec<(String, String)>) = match kind {
        RegionFormat::Indent(IndentBlock {
            amount,
            wrap,
            center,
            layout,
            styles,
        }) => {
            let mut kvs = vec![("amount".to_owned(), amount.to_string())];
            if let Some(w) = wrap {
                kvs.push(("wrap".to_owned(), w.to_string()));
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
            // #78 co-applied styles — a space-joined `modifiers` value (the
            // Format identity tags, canonical order), mirroring the HTML
            // class list. Open-ended: a new style adds a token, not a kv key.
            let modifiers: Vec<&str> = styles.iter_formats().map(Format::as_json_tag).collect();
            if !modifiers.is_empty() {
                kvs.push(("modifiers".to_owned(), modifiers.join(" ")));
            }
            ("container-indent", kvs)
        }
        RegionFormat::Warichu => ("container-warichu", Vec::new()),
        RegionFormat::Framed => ("container-keigakomi", Vec::new()),
        RegionFormat::AlignEnd { offset } => (
            "container-align-end",
            vec![("offset".to_owned(), offset.to_string())],
        ),
        RegionFormat::Bouten { kind, position } => {
            let mut kvs = vec![(
                "variant".to_owned(),
                roman_slug(kind.keyword()).unwrap_or("unknown").to_owned(),
            )];
            if matches!(position, BoutenPosition::Left) {
                kvs.push(("position".to_owned(), "left".to_owned()));
            }
            ("container-bouten", kvs)
        }
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        to_pandoc(&doc.parse_owned()).blocks
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
    fn explicit_ruby_projects_as_ruby_span() {
        let blocks = project("｜青梅《おうめ》\n");
        let (_, inner) = find_span(&blocks, "ruby").expect("ruby span");
        assert_eq!(inner.len(), 2, "ruby has base + reading children");
    }

    #[test]
    fn left_ruby_projects_as_ruby_span() {
        let blocks = project("未［＃「未」の左に「ザル」のルビ］んとす。\n");
        let (_, inner) = find_span(&blocks, "ruby").expect("left ruby span");
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
    fn side_note_projects_base_and_note_subspans() {
        let blocks = project("未来［＃「未来」の左に「みらい」の注記］を見る。\n");
        let (_, inner) = find_span(&blocks, "sidenote").expect("sidenote span");
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
        // non-adjacent `明治３３年［＃…］` form is `Referenced` — covered by
        // `referenced_forward_is_not_double_projected`.)
        let blocks = project("明治３３［＃「３３」は縦中横］年に。\n");
        let (_, inner) = find_span(&blocks, "tate-chu-yoko").expect("tcy span");
        assert_eq!(
            inner,
            &[Inline::Str("３３".to_owned())],
            "tcy embedded text"
        );
    }

    #[test]
    fn referenced_forward_is_not_double_projected() {
        // #231: a non-adjacent forward (`Referenced`) keeps its target literal
        // in the upstream run, so the projection must render it once with no
        // styled span. Projecting `f.target` too would double 青空 — the same
        // root cause as the HTML bug #228.
        let blocks = project("青空の下を歩く［＃「青空」に傍点］");
        assert!(
            find_span(&blocks, "bouten").is_none(),
            "Referenced bouten must not emit a styled span: {blocks:?}"
        );
        let Some(Block::Para(inlines)) = blocks.first() else {
            panic!("expected a single Para, got {blocks:?}");
        };
        assert_eq!(
            inlines.as_slice(),
            &[Inline::Str("青空の下を歩く".to_owned())],
            "the literal 青空 must appear exactly once, upstream"
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
        match inner {
            [Inline::Str(s)] => assert!(
                s.starts_with("Char("),
                "resolved gaiji renders the debug Char(...) form, got {s:?}"
            ),
            other => panic!("expected single Str for resolved gaiji, got {other:?}"),
        }
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
    fn angle_quote_projects_to_span() {
        let blocks = project("≪重要≫な記述。\n");
        let (_, inner) = find_span(&blocks, "angle-quote").expect("angle-quote span");
        assert_eq!(inner, &[Inline::Str("重要".to_owned())], "angle-quote text");
    }

    #[test]
    fn kaeriten_re_mark_projects_to_span() {
        let blocks = project("天［＃（レ）］地\n");
        let (attr, _) = find_span(&blocks, "kaeriten").expect("kaeriten span");
        assert_eq!(kv(attr, "mark"), Some("（レ）"), "kaeriten mark text");
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
    }

    // -----------------------------------------------------------------
    // Directive fallthrough (kind slug arms)
    // -----------------------------------------------------------------

    #[test]
    fn unknown_annotation_kind_slug() {
        // `［＃見出し］` is not a dedicated node → generic Unknown annotation.
        let blocks = project("［＃見出し］序章［＃見出し終わり］\n");
        let (attr, _) = find_span(&blocks, "annotation").expect("annotation span");
        assert_eq!(kv(attr, "kind"), Some("unknown"), "unknown annotation kind");
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
    fn other_annotation_kind_slug() {
        // `［＃割り注］` inline classifies as a non-Unknown, non-correction
        // annotation → the `other` slug arm.
        let blocks = project("［＃割り注］上の段／下の段［＃割り注終わり］\n");
        let (attr, _) = find_span(&blocks, "annotation").expect("annotation span");
        assert_eq!(kv(attr, "kind"), Some("other"), "割り注 → other kind");
    }

    // -----------------------------------------------------------------
    // Emphasis fallthrough (the `other` arm in dispatch_inline_node)
    // -----------------------------------------------------------------

    #[test]
    fn emphasis_font_size_falls_through_to_debug_span() {
        // Forward-reference emphasis has no dedicated projection — it lands
        // in the catch-all `other` arm as a plain-attr debug Span.
        let blocks = project("甲［＃「甲」は2段階大きな文字］\n");
        let para = match &blocks[0] {
            Block::Para(inlines) => inlines,
            other => panic!("expected Para, got {other:?}"),
        };
        let dbg = para
            .iter()
            .find_map(|i| match i {
                Inline::Span(attr, inner) if attr.1.is_empty() => Some(inner),
                _ => None,
            })
            .expect("debug fallback span with empty class list");
        match dbg.as_slice() {
            [Inline::Str(s)] => assert!(
                s.contains("ForwardFormat") && s.contains("FontSize"),
                "fallback renders the forward format's Debug form, got {s:?}"
            ),
            other => panic!("expected single debug Str, got {other:?}"),
        }
    }

    #[test]
    fn emphasis_superscript_falls_through_to_debug_span() {
        let blocks = project("ｅ２［＃「２」は上付き小文字］\n");
        let json = serde_json::to_string(&blocks).expect("serialise blocks");
        assert!(
            json.contains("SuperScript"),
            "superscript emphasis lands as a debug Str: {json}"
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
    fn sashie_keyword_form_with_caption_uses_caption_alt() {
        let blocks = project("ある日、［＃挿絵（cover.png）「図一」入る］その地に至る。\n");
        let img = find_image(&blocks).expect("sashie image");
        assert_eq!(
            img.1,
            &[Inline::Str("図一".to_owned())],
            "caption becomes alt text"
        );
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

    /// Find the first `Image` inline in any `Para` block.
    fn find_image(blocks: &[Block]) -> Option<(&Attr, &[Inline], &(String, String))> {
        blocks.iter().find_map(|b| match b {
            Block::Para(inlines) => inlines.iter().find_map(|i| match i {
                Inline::Image(attr, alt, target) => Some((attr, alt.as_slice(), target)),
                _ => None,
            }),
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
        let (attr, _) = find_div(&blocks, "container-bouten").expect("bouten range div");
        assert_eq!(kv(attr, "variant"), Some("goma"), "default bouten variant");
        assert!(
            kv(attr, "position").is_none(),
            "right-side range omits the position kv"
        );
    }

    #[test]
    fn bouten_range_left_position_kv() {
        let blocks = project("本文［＃左に傍線］丙《へい》［＃左に傍線終わり］。");
        let (attr, _) = find_div(&blocks, "container-bouten").expect("bouten range div");
        assert_eq!(kv(attr, "variant"), Some("bosen"), "傍線 variant slug");
        assert_eq!(kv(attr, "position"), Some("left"), "left-side range kv");
    }

    #[test]
    fn unknown_container_falls_through_to_unknown_class() {
        // 段組 (columns) has no dedicated container_attr arm → `container-unknown`.
        let blocks =
            project("前文。\n［＃ここから2段組み］\n左右。\n［＃ここで段組み終わり］\n後文。\n");
        let div = find_div(&blocks, "container-unknown");
        assert!(div.is_some(), "columns → container-unknown div: {blocks:?}");
    }

    #[test]
    fn table_container_falls_through_to_unknown_class() {
        let blocks = project("［＃ここから表］\n項目\u{3000}値\n［＃ここで表終わり］\n");
        assert!(
            find_div(&blocks, "container-unknown").is_some(),
            "table → container-unknown: {blocks:?}"
        );
    }

    #[test]
    fn bold_block_container_falls_through_to_unknown_class() {
        let blocks = project("［＃ここから太字］\n強調する段落。\n［＃ここで太字終わり］\n");
        assert!(
            find_div(&blocks, "container-unknown").is_some(),
            "bold block → container-unknown: {blocks:?}"
        );
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
        let pandoc = to_pandoc(&doc.parse_owned());
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
        let inline = line_inline(LineFormat::Indent { amount: 4 });
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
    fn line_inline_keigakomi_is_empty_marker() {
        match line_inline(LineFormat::Framed) {
            Inline::Span(attr, inner) => {
                assert!(has_class(&attr, "keigakomi"), "keigakomi class");
                assert!(inner.is_empty(), "keigakomi marker span is empty");
            }
            other => panic!("expected Span, got {other:?}"),
        }
    }

    #[test]
    fn warichu_inline_builder_wraps_upper_and_lower() {
        // Build the owned warichu payload directly via a store (the `／`-split
        // upper / lower form is not reachable as an inline leaf).
        let mut store = NodeStore::new();
        let upper = ContentOwned::Plain(store.intern("上"));
        let lower = ContentOwned::Plain(store.intern("下"));
        let w = WarichuOwned { upper, lower };
        match warichu_inline(w, &store) {
            Inline::Span(attr, inner) => {
                assert!(has_class(&attr, "warichu"), "warichu class");
                assert_eq!(inner.len(), 2, "warichu wraps upper + lower");
                assert!(
                    matches!(&inner[0], Inline::Span(a, _) if has_class(a, "warichu-upper")),
                    "first child is warichu-upper: {inner:?}"
                );
                assert!(
                    matches!(&inner[1], Inline::Span(a, _) if has_class(a, "warichu-lower")),
                    "second child is warichu-lower: {inner:?}"
                );
            }
            other => panic!("expected Span, got {other:?}"),
        }
    }

    #[test]
    fn bouten_position_slug_covers_left_and_unknown() {
        assert_eq!(bouten_position_slug(BoutenPosition::Right), "right");
        assert_eq!(bouten_position_slug(BoutenPosition::Left), "left");
    }

    #[test]
    fn annotation_kind_slug_covers_all_named_arms() {
        assert_eq!(annotation_kind_slug(DirectiveKind::Unknown), "unknown");
        assert_eq!(annotation_kind_slug(DirectiveKind::Sic), "sic");
        assert_eq!(
            annotation_kind_slug(DirectiveKind::BaseTextVariant),
            "base-text-variant"
        );
        assert_eq!(
            annotation_kind_slug(DirectiveKind::InvalidRubySpan),
            "invalid-ruby-span"
        );
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
        let text = store.push_contents(&[ContentOwned::Plain(text_id)]);
        let heading = HeadingOwned {
            kind: HeadingKind::Small,
            style: HeadingStyle::SameLine,
            text,
        };
        match aozora_heading_block(heading, &store) {
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
}
