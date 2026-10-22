#![expect(
    clippy::expect_used,
    reason = "the in-memory capped writer is infallible"
)]

//! Serializes normalized text and semantic nodes back to Aozora source.

use core::fmt::{self, Write};

use crate::pipeline::{has_long_rule_line, isolate_decorative_rules, lex};
use crate::render::spelling::source::{
    NewlineCappedWriter, TrackingWriter, emit_container_close, emit_container_open, emit_line,
    emit_section_break, heading_level_word, heading_style_keyword,
};
use crate::render::walk::{SentinelKind, WalkSink, walk};
use crate::spec::Diagnostic;
use crate::syntax::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardFormat, ForwardPayload, Gaiji,
    GaijiCanonicalOwned, Heading, HeadingHint, Illustration, Kaeriten, LexOutput, MarginNote, Node,
    NodeRef, NodeStore, Ruby, Segment, content_ruby_base_class, gaiji_ruby_base_class,
};
use crate::syntax::degraded::degraded_directive;
use crate::syntax::format::ForwardOrigin;
use crate::syntax::lint::canonical_directive;
use crate::syntax::{
    AccentMark, BoutenPosition, DirectiveKind, EnclosureKind, ForwardAttr, RubyBaseClass, RubySide,
    ruby_base_class,
};

/// Directive normalization levels from ADR-0022 and ADR-0026.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectiveNormalization {
    /// Preserve directive spelling and render unknown directives as hidden markers.
    #[default]
    Off,
    /// Replace verified spelling variants with canonical notation.
    Canonical,
    /// Also apply lossy reductions for temporary rendering, as defined in ADR-0026.
    Degraded,
}

/// Options for serializing a snapshot to Aozora source.
///
/// Defaults to [`DirectiveNormalization::Off`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SerializeOptions {
    pub(crate) directives: DirectiveNormalization,
}

impl SerializeOptions {
    /// Select how to normalize directives.
    #[must_use]
    pub fn directives(mut self, level: DirectiveNormalization) -> Self {
        self.directives = level;
        self
    }
}

#[must_use]
pub(crate) fn serialize(out: &LexOutput) -> String {
    serialize_with(out, SerializeOptions::default())
}

#[must_use]
pub(crate) fn serialize_with(out: &LexOutput, opts: SerializeOptions) -> String {
    if opts.directives == DirectiveNormalization::Off && requires_verbatim_recovery(out) {
        return out.sanitized.to_string();
    }
    let first = serialize_pass(out, opts);
    match opts.directives {
        DirectiveNormalization::Off => first,
        DirectiveNormalization::Canonical | DirectiveNormalization::Degraded => {
            serialize_pass(&lex(&first), SerializeOptions::default())
        }
    }
}

fn serialize_pass(out: &LexOutput, opts: SerializeOptions) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_into_pass(out, &mut s, opts).expect("writing to NewlineCappedWriter never fails");
    let raw = s.into_string();
    if has_long_rule_line(&raw) {
        isolate_decorative_rules(&raw)
    } else {
        raw
    }
}

#[cfg(test)]
pub(crate) fn serialize_into<W: Write>(out: &LexOutput, writer: &mut W) -> fmt::Result {
    serialize_into_with(out, writer, SerializeOptions::default())
}

#[cfg(test)]
pub(crate) fn serialize_into_with<W: Write>(
    out: &LexOutput,
    writer: &mut W,
    opts: SerializeOptions,
) -> fmt::Result {
    if opts.directives == DirectiveNormalization::Off && requires_verbatim_recovery(out) {
        return writer.write_str(&out.sanitized);
    }
    if opts.directives != DirectiveNormalization::Off {
        return writer.write_str(&serialize_with(out, opts));
    }
    serialize_into_pass(out, writer, opts)
}

pub(crate) fn requires_verbatim_recovery(out: &LexOutput) -> bool {
    out.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic,
            Diagnostic::UnclosedBracket { .. }
                | Diagnostic::MismatchedContainerClose { .. }
                | Diagnostic::NestedRuby { .. }
        )
    })
}

fn serialize_into_pass<W: Write>(
    out: &LexOutput,
    writer: &mut W,
    opts: SerializeOptions,
) -> fmt::Result {
    let mut tracking = TrackingWriter::new(writer);
    let mut sink = SerializeSink {
        store: &out.store,
        out: &mut tracking,
        directives: opts.directives,
        predecessor: RubyPredecessor::Boundary,
    };
    walk(out, &mut sink)
}

struct SerializeSink<'a, W: Write> {
    store: &'a NodeStore,
    out: &'a mut TrackingWriter<W>,
    directives: DirectiveNormalization,
    predecessor: RubyPredecessor,
}

impl<W: Write> WalkSink for SerializeSink<'_, W> {
    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)?;
        if let Some(ch) = text.chars().next_back() {
            self.predecessor = RubyPredecessor::from_char(ch);
        }
        Ok(())
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(n))
            | (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => {
                emit_aozora(
                    n,
                    self.store,
                    self.out,
                    EmitContext {
                        directives: self.directives,
                        predecessor: self.predecessor,
                    },
                )?;
                self.predecessor = RubyPredecessor::after_node(n, self.store);
                Ok(())
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                emit_container_open(open, self.out)?;
                self.predecessor = RubyPredecessor::Boundary;
                Ok(())
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                emit_container_close(close, self.out)?;
                self.predecessor = RubyPredecessor::Boundary;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RubyPredecessor {
    Boundary,
    Bar,
    Text(RubyBaseClass),
    Gaiji(Option<RubyBaseClass>),
}

#[derive(Debug, Clone, Copy)]
struct EmitContext {
    directives: DirectiveNormalization,
    predecessor: RubyPredecessor,
}

impl RubyPredecessor {
    fn from_char(ch: char) -> Self {
        if ch == '｜' {
            Self::Bar
        } else {
            ruby_base_class(ch).map_or(Self::Boundary, Self::Text)
        }
    }

    fn after_node(node: Node, store: &NodeStore) -> Self {
        match node {
            Node::Gaiji(gaiji) => Self::Gaiji(gaiji_ruby_base_class(gaiji, store)),
            Node::Format(format) if matches!(format.origin, ForwardOrigin::Detached) => {
                Self::after_plain_content_range(format.target, store)
            }
            Node::Ruby(_)
            | Node::Format(_)
            | Node::MarginNote(_)
            | Node::Line(_)
            | Node::PageBreak
            | Node::SectionBreak(_)
            | Node::BodyEnd
            | Node::ForcedBreak
            | Node::Directive(_)
            | Node::Kaeriten(_)
            | Node::AngleQuote(_)
            | Node::Illustration(_)
            | Node::Heading(_)
            | Node::HeadingHint(_) => Self::Boundary,
        }
    }

    fn after_plain_content_range(range: ContentRange, store: &NodeStore) -> Self {
        store
            .resolve_content_range(range)
            .iter()
            .filter_map(|content| Self::after_plain_content(*content, store))
            .next_back()
            .unwrap_or(Self::Boundary)
    }

    fn after_plain_content(content: Content, store: &NodeStore) -> Option<Self> {
        match content {
            Content::Plain(id) => store
                .resolve_str(id)
                .chars()
                .next_back()
                .map(Self::from_char),
            Content::Segments(range) => store
                .resolve_seg_range(range)
                .iter()
                .filter_map(|segment| match *segment {
                    Segment::Text(id) => store
                        .resolve_str(id)
                        .chars()
                        .next_back()
                        .map(Self::from_char),
                    Segment::Gaiji(gaiji) => store
                        .resolve_str(gaiji.hint)
                        .chars()
                        .next_back()
                        .map(Self::from_char),
                    Segment::Directive(directive) => store
                        .resolve_str(directive.raw)
                        .chars()
                        .next_back()
                        .map(Self::from_char),
                    Segment::Node(node) => Some(Self::after_node(node, store)),
                })
                .next_back(),
        }
    }
}

fn emit_aozora<W: Write>(
    node: Node,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
    context: EmitContext,
) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_ruby(&r, store, out, context.predecessor),
        Node::Format(f) => emit_format(&f, store, out),
        Node::Gaiji(g) => emit_gaiji(&g, store, out),
        Node::Kaeriten(k) => emit_kaeriten(k, store, out),
        Node::Directive(a) => emit_annotation(a, store, out, context.directives),
        Node::AngleQuote(d) => emit_angle_quote(d, store, out),
        Node::MarginNote(s) => emit_side_note(&s, store, out),
        Node::PageBreak => out.write_str("［＃改ページ］"),
        Node::BodyEnd => out.write_str("［＃本文終わり］"),
        Node::ForcedBreak => out.write_str("［＃改行］"),
        Node::SectionBreak(kind) => emit_section_break(kind, out),
        Node::Line(lf) => emit_line(lf, out),
        Node::Illustration(s) => emit_sashie(&s, store, out),
        Node::HeadingHint(h) => emit_heading_hint(h, store, out),
        Node::Heading(h) => emit_aozora_heading(&h, store, out),
    }
}

fn emit_content_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => out.write_str(store.resolve_str(id))?,
                    Segment::Gaiji(g) => emit_gaiji(&g, store, out)?,
                    Segment::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    Segment::Node(node) => emit_nested_node(node, store, out)?,
                }
            }
            Ok(())
        }
    }
}

fn emit_content_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        emit_content_one(*c, store, out)?;
    }
    Ok(())
}

fn emit_content_as_plain_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => out.write_str(store.resolve_str(id))?,
                    Segment::Gaiji(g) => out.write_str(store.resolve_str(g.hint))?,
                    Segment::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    Segment::Node(node) => emit_nested_node(node, store, out)?,
                }
            }
            Ok(())
        }
    }
}

fn emit_nested_node<W: Write>(node: Node, store: &NodeStore, out: &mut W) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_nested_ruby(&r, store, out),
        Node::Format(f) => emit_format(&f, store, out),
        Node::Gaiji(g) => emit_gaiji(&g, store, out),
        Node::Kaeriten(k) => emit_kaeriten(k, store, out),
        Node::Directive(a) => emit_annotation(a, store, out, DirectiveNormalization::Off),
        Node::AngleQuote(d) => emit_angle_quote(d, store, out),
        Node::MarginNote(s) => emit_side_note(&s, store, out),
        Node::PageBreak => out.write_str("［＃改ページ］"),
        Node::BodyEnd => out.write_str("［＃本文終わり］"),
        Node::ForcedBreak => out.write_str("［＃改行］"),
        Node::SectionBreak(kind) => emit_section_break(kind, out),
        Node::Line(lf) => emit_line(lf, out),
        Node::Illustration(s) => emit_sashie(&s, store, out),
        Node::HeadingHint(h) => emit_heading_hint(h, store, out),
        Node::Heading(h) => emit_aozora_heading(&h, store, out),
    }
}

// mutants::skip: `node_is_content_segment` prevents allocator-built content from reaching this defensive emitter.
#[cfg_attr(test, mutants::skip)]
fn emit_nested_ruby<W: Write>(r: &Ruby, store: &NodeStore, out: &mut W) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        emit_content_range(r.base, store, out)?;
        out.write_str("［＃「")?;
        emit_content_range(r.base, store, out)?;
        out.write_str("」の左に「")?;
        emit_content_range(r.reading, store, out)?;
        return out.write_str("」のルビ］");
    }
    out.write_char('｜')?;
    emit_content_range(r.base, store, out)?;
    out.write_char('《')?;
    emit_content_range(r.reading, store, out)?;
    out.write_char('》')
}

fn emit_content_as_plain_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        emit_content_as_plain_one(*c, store, out)?;
    }
    Ok(())
}

fn emit_ruby<W: Write>(
    r: &Ruby,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
    predecessor: RubyPredecessor,
) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        emit_content_range(r.base, store, out)?;
        out.write_str("［＃「")?;
        emit_content_range(r.base, store, out)?;
        out.write_str("」の左に「")?;
        emit_content_range(r.reading, store, out)?;
        return out.write_str("」のルビ］");
    }
    let predecessor = if out.last() == Some('｜') {
        RubyPredecessor::Bar
    } else {
        predecessor
    };
    if ruby_needs_bar(store.resolve_content_range(r.base), predecessor, store) {
        out.write_char('｜')?;
    }
    emit_content_range(r.base, store, out)?;
    out.write_char('《')?;
    emit_content_range(r.reading, store, out)?;
    out.write_char('》')
}

fn ruby_needs_bar(base_run: &[Content], predecessor: RubyPredecessor, store: &NodeStore) -> bool {
    let all_gaiji = matches!(base_run, [Content::Segments(range)] if {
        let segments = store.resolve_seg_range(*range);
        !segments.is_empty()
            && segments
                .iter()
                .all(|segment| matches!(segment, Segment::Gaiji(_)))
    });
    let base_class = content_ruby_base_class(base_run, store);
    if all_gaiji {
        return match predecessor {
            RubyPredecessor::Boundary => false,
            RubyPredecessor::Bar | RubyPredecessor::Gaiji(_) => true,
            RubyPredecessor::Text(class) => base_class == Some(class),
        };
    }
    let Some(base_class) = base_class else {
        return true;
    };
    match predecessor {
        RubyPredecessor::Bar => true,
        RubyPredecessor::Text(class) | RubyPredecessor::Gaiji(Some(class)) => class == base_class,
        RubyPredecessor::Boundary | RubyPredecessor::Gaiji(None) => false,
    }
}

fn emit_format<W: Write>(f: &ForwardFormat, store: &NodeStore, out: &mut W) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Reclaimed | ForwardOrigin::Detached) {
        if matches!(f.payload, ForwardPayload::QuotedTarget(_)) {
            emit_content_range(f.target, store, out)?;
        } else {
            emit_content_as_plain_range(f.target, store, out)?;
        }
    }
    if matches!(f.origin, ForwardOrigin::Detached) {
        return Ok(());
    }
    if let ForwardAttr::Bouten { kind, position } = f.attr {
        out.write_str("［＃")?;
        if let ForwardPayload::QuotedTarget(id) = f.payload {
            out.write_char('「')?;
            out.write_str(store.resolve_str(id))?;
            out.write_char('」')?;
        } else {
            emit_bouten_targets(store.resolve_content_range(f.target), store, out)?;
        }
        match position {
            BoutenPosition::Right => out.write_char('に')?,
            BoutenPosition::Left => out.write_str("の左に")?,
            BoutenPosition::Both => out.write_str("の両側に")?,
        }
        out.write_str(kind.keyword())?;
        return out.write_char('］');
    }
    if matches!(f.attr, ForwardAttr::Framed(EnclosureKind::Box)) {
        out.write_str("［＃「")?;
        emit_forward_target_spelling(f, store, out)?;
        return out.write_str("」は「□」囲み］");
    }
    if matches!(f.attr, ForwardAttr::AccentDot) {
        out.write_str("［＃")?;
        if let ForwardPayload::AccentBody(id) = f.payload {
            out.write_str(store.resolve_str(id))?;
        }
        return out.write_char('］');
    }
    out.write_str("［＃「")?;
    emit_forward_target_spelling(f, store, out)?;
    out.write_str("」は")?;
    if let ForwardAttr::FontSize(shift) = f.attr {
        let word = if shift.larger() {
            "大きな"
        } else {
            "小さな"
        };
        write!(out, "{}段階{word}文字", shift.magnitude())?;
    } else if let ForwardAttr::AlignEnd { offset } = f.attr {
        if offset == 0 {
            out.write_str("地付き")?;
        } else {
            write!(out, "文末より{offset}字上げ揃え")?;
        }
    } else if let ForwardAttr::Accent(mark) = f.attr {
        out.write_str(accent_suffix(mark))?;
    } else {
        out.write_str(f.attr.keyword())?;
    }
    out.write_char('］')
}

fn emit_forward_target_spelling<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    if let ForwardPayload::QuotedTarget(id) = f.payload {
        out.write_str(store.resolve_str(id))
    } else {
        emit_content_as_plain_range(f.target, store, out)
    }
}

const fn accent_suffix(mark: AccentMark) -> &'static str {
    match mark {
        AccentMark::Acute => "アクサン（´）付き",
        AccentMark::Grave => "アクサン（｀）付き",
        AccentMark::Umlaut => "ウムラウト（¨）付き",
    }
}

fn emit_bouten_targets<W: Write>(run: &[Content], store: &NodeStore, out: &mut W) -> fmt::Result {
    if let [Content::Plain(id)] = run {
        out.write_char('「')?;
        out.write_str(store.resolve_str(*id))?;
        return out.write_char('」');
    }
    let mut any = false;
    for c in run {
        if let Content::Segments(seg_range) = c {
            for seg in store.resolve_seg_range(*seg_range) {
                if let Segment::Text(id) = *seg {
                    let t = store.resolve_str(id);
                    for part in t.split('、').filter(|p| !p.is_empty()) {
                        out.write_char('「')?;
                        out.write_str(part)?;
                        out.write_char('」')?;
                        any = true;
                    }
                }
            }
        }
    }
    if !any {
        out.write_char('「')?;
        out.write_char('」')?;
    }
    Ok(())
}

fn emit_gaiji<W: Write>(g: &Gaiji, store: &NodeStore, out: &mut W) -> fmt::Result {
    if !g.standalone {
        out.write_char('※')?;
    }
    out.write_str("［＃")?;
    let hint = store.resolve_str(g.hint);
    if hint.contains(['「', '」']) {
        out.write_str(hint)?;
    } else {
        out.write_char('「')?;
        out.write_str(hint)?;
        out.write_char('」')?;
    }
    if gaiji_has_mencode(g.canonical) {
        if g.mencode_separator {
            out.write_char('、')?;
        }
        write_gaiji_mencode(g.canonical, store, out)?;
    }
    out.write_char('］')
}

const fn gaiji_has_mencode(c: GaijiCanonicalOwned) -> bool {
    !matches!(c, GaijiCanonicalOwned::Unresolved { mencode: None })
}

fn write_gaiji_mencode<W: Write>(
    c: GaijiCanonicalOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match c {
        GaijiCanonicalOwned::MenKuTen(m) => write!(out, "{m}"),
        GaijiCanonicalOwned::Unicode(ch) => write!(out, "U+{:04X}", ch as u32),
        GaijiCanonicalOwned::Unresolved { mencode } => {
            mencode.map_or(Ok(()), |id| out.write_str(store.resolve_str(id)))
        }
    }
}

fn emit_kaeriten<W: Write>(k: Kaeriten, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(store.resolve_str(k.mark))?;
    out.write_char('］')
}

fn emit_annotation<W: Write>(
    a: Directive,
    store: &NodeStore,
    out: &mut W,
    directives: DirectiveNormalization,
) -> fmt::Result {
    let raw = store.resolve_str(a.raw);
    if directives != DirectiveNormalization::Off && a.kind == DirectiveKind::NonCanonical {
        let body = raw
            .strip_prefix("［＃")
            .and_then(|s| s.strip_suffix('］'))
            .unwrap_or(raw)
            .trim();
        let resolved = canonical_directive(body).or_else(|| {
            (directives == DirectiveNormalization::Degraded)
                .then(|| degraded_directive(body))
                .flatten()
        });
        if let Some(canonical) = resolved {
            out.write_str("［＃")?;
            out.write_str(canonical.as_ref())?;
            return out.write_char('］');
        }
    }
    out.write_str(raw)
}

fn emit_angle_quote<W: Write>(d: AngleQuote, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_char('≪')?;
    emit_content_range(d.content, store, out)?;
    out.write_char('≫')
}

fn emit_side_note<W: Write>(s: &MarginNote, store: &NodeStore, out: &mut W) -> fmt::Result {
    let (connector, suffix) = s.kind.serialize_affixes();
    emit_content_range(s.base, store, out)?;
    out.write_str("［＃「")?;
    emit_content_range(s.base, store, out)?;
    out.write_str(connector)?;
    emit_content_range(s.note, store, out)?;
    out.write_str(suffix)
}

fn emit_sashie<W: Write>(s: &Illustration, store: &NodeStore, out: &mut W) -> fmt::Result {
    if let Some(caption @ Content::Segments(_)) = s.caption
        && s.description.is_none()
    {
        out.write_str("［＃「")?;
        emit_content_one(caption, store, out)?;
        out.write_str("」のキャプション付きの挿絵（")?;
        out.write_str(store.resolve_str(s.file))?;
        if let Some(dims) = s.dimensions {
            out.write_char('、')?;
            out.write_str(store.resolve_str(dims))?;
        }
        return out.write_str("）入る］");
    }
    out.write_str("［＃")?;
    if let Some(description) = s.description {
        out.write_str(store.resolve_str(description))?;
    } else {
        out.write_str("挿絵")?;
        if let Some(number) = s.number {
            out.write_str(store.resolve_str(number))?;
        }
    }
    out.write_char('（')?;
    out.write_str(store.resolve_str(s.file))?;
    if let Some(dims) = s.dimensions {
        out.write_char('、')?;
        out.write_str(store.resolve_str(dims))?;
    }
    out.write_char('）')?;
    if let Some(caption) = s.caption {
        out.write_char('「')?;
        emit_content_one(caption, store, out)?;
        out.write_char('」')?;
    }
    out.write_str("入る］")
}

fn emit_heading_hint<W: Write>(h: HeadingHint, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(store.resolve_str(h.target))?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.level))?;
    out.write_str("］")
}

fn emit_aozora_heading<W: Write>(h: &Heading, store: &NodeStore, out: &mut W) -> fmt::Result {
    emit_content_range(h.text, store, out)?;
    out.write_str("\n［＃「")?;
    emit_content_range(h.text, store, out)?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.kind))?;
    out.write_str("］")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::pipeline::lex;
    use crate::render::render_html;
    use crate::syntax::alloc::Allocator;
    use crate::syntax::{
        BoutenKind, HeadingKind, HeadingStyle, MarginNoteKind, RegionClose, RegionFormat,
    };

    fn assert_parity(src: &str) {
        let first = serialize(&lex(src));
        let second = serialize(&lex(&first));
        assert_eq!(first, second, "serialize fixed point diverged for {src:?}");
    }

    #[test]
    fn serialize_is_fixed_point_across_node_kinds() {
        for src in [
            "plain text",
            "｜青梅《おうめ》",
            "頃｜青梅《おうめ》",
            "再読［＃「再読」の左に「さい」のルビ］",
            "底本「青空」［＃「青空」の左に「注記」の注記］",
            "可哀想［＃「可哀想」に傍点］",
            "甲乙［＃「甲」「乙」に傍点］",
            "重要［＃「重要」は太字］",
            "X［＃「X」は3段階大きな文字］",
            "※［＃「○○」、第3水準1-85-54］",
            "一二［＃レ］",
            "≪重要≫",
            "見出し\n［＃「見出し」は大見出し］",
            "［＃挿絵（fig.png、横480×縦640）入る］",
            "［＃改ページ］",
            "本編［＃本文終わり］",
            "行頭［＃改行］行末",
            "［＃ここから2字下げ］\nA\n［＃ここで字下げ終わり］",
            "段落の文\n――――――――――――\n｜青梅《おうめ》の続き\n",
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》",
            "｜A※［＃「特のへん＋廴＋聿」、第3水準1-87-71］漢《r》",
            "｜陀※［＃「架空の字」］多《r》",
            "川※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《r》",
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀※［＃「特のへん＋廴＋聿」、第3水準1-87-71］多《r》",
            "文｜｜※［＃「目＋爭」、第3水準1-88-85］《みは》る",
            "元｜※［＃「目＋爭」、第3水準1-88-85］《みは》る",
            "※［＃「目＋爭」、第3水準1-88-85］｜※［＃「目＋爭」、第3水準1-88-85］《みは》",
            "※［＃「目＋爭」、第3水準1-88-85］｜※［＃「目＋爭」、第3水準1-88-85］陀《みは》",
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀《かん※［＃「特のへん＋廴＋聿」、第3水準1-87-71］》",
        ] {
            assert_parity(src);
        }
    }

    #[test]
    fn streamed_plain_explicit_ruby_base_is_not_duplicated() {
        let source = "｜〔canapé〕《長椅子》";
        let parsed = lex(source);
        let [entry] = parsed.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", parsed.source_nodes);
        };

        assert_eq!(entry.source_span.slice(source), source);
        let first = serialize(&parsed);
        let second = serialize(&lex(&first));
        assert_eq!(first, source);
        assert_eq!(second, first);
        assert_eq!(render_html(&parsed).matches("canapé").count(), 1);
    }

    #[test]
    fn structured_ruby_serialization_preserves_rendered_scope() {
        for source in [
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》",
            "｜A※［＃「特のへん＋廴＋聿」、第3水準1-87-71］漢《r》",
            "｜陀※［＃「架空の字」］多《r》",
            "川※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《r》",
            "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀※［＃「特のへん＋廴＋聿」、第3水準1-87-71］多《r》",
            "文｜｜※［＃「目＋爭」、第3水準1-88-85］《みは》る",
            "元｜※［＃「目＋爭」、第3水準1-88-85］《みは》る",
            "※［＃「目＋爭」、第3水準1-88-85］｜※［＃「目＋爭」、第3水準1-88-85］《みは》",
            "※［＃「目＋爭」、第3水準1-88-85］｜※［＃「目＋爭」、第3水準1-88-85］陀《みは》",
        ] {
            let parsed = lex(source);
            let canonical = serialize(&parsed);
            let reparsed = lex(&canonical);
            assert_eq!(
                render_html(&parsed),
                render_html(&reparsed),
                "rendered scope changed for {source:?} via {canonical:?}"
            );
            assert_eq!(
                serialize(&reparsed),
                canonical,
                "second serialization changed {canonical:?}"
            );
        }
    }

    #[test]
    fn detached_format_tail_keeps_the_following_ruby_explicit() {
        for segmented in [false, true] {
            let mut allocator = Allocator::new();
            let target = if segmented {
                let prefix = allocator.seg_text("前");
                let tail = allocator.seg_text("漢");
                allocator.content_segments(&[prefix, tail])
            } else {
                allocator.content_plain("漢")
            };
            let detached =
                allocator.forward_format(ForwardAttr::Bold, target, ForwardOrigin::Detached);
            let base = allocator.content_plain("字");
            let reading = allocator.content_plain("じ");
            let ruby = allocator.ruby(base, reading);
            let store = allocator.into_store();

            let mut source = String::new();
            let () = {
                let mut writer = TrackingWriter::new(&mut source);
                let mut sink = SerializeSink {
                    store: &store,
                    out: &mut writer,
                    directives: DirectiveNormalization::Off,
                    predecessor: RubyPredecessor::Boundary,
                };
                sink.on_node(SentinelKind::Inline, NodeRef::Inline(detached))
                    .expect("serialize into String is infallible");
                sink.on_node(SentinelKind::Inline, NodeRef::Inline(ruby))
                    .expect("serialize into String is infallible");
            };

            let expected = if segmented {
                "前漢｜字《じ》"
            } else {
                "漢｜字《じ》"
            };
            assert_eq!(source, expected);
            assert_eq!(serialize(&lex(&source)), source);
        }
    }

    #[test]
    fn referenced_format_is_a_boundary_for_the_following_ruby() {
        let mut allocator = Allocator::new();
        let target = allocator.content_plain("漢");
        let referenced =
            allocator.forward_format(ForwardAttr::Bold, target, ForwardOrigin::Referenced);
        let base = allocator.content_plain("字");
        let reading = allocator.content_plain("じ");
        let ruby = allocator.ruby(base, reading);
        let store = allocator.into_store();

        let mut source = String::new();
        {
            let mut writer = TrackingWriter::new(&mut source);
            let mut sink = SerializeSink {
                store: &store,
                out: &mut writer,
                directives: DirectiveNormalization::Off,
                predecessor: RubyPredecessor::Boundary,
            };
            sink.on_node(SentinelKind::Inline, NodeRef::Inline(referenced))
                .expect("serialize into String is infallible");
            sink.on_node(SentinelKind::Inline, NodeRef::Inline(ruby))
                .expect("serialize into String is infallible");
        };

        assert_eq!(source, "［＃「漢」は太字］字《じ》");
        assert_eq!(serialize(&lex(&source)), source);
    }

    #[test]
    fn different_text_class_before_ruby_does_not_force_a_bar() {
        let source = "あ字《じ》";
        assert_eq!(serialize(&lex(source)), source);
    }

    #[test]
    fn normalized_serializer_reflows_block_directives_before_returning() {
        let opts = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
        let out = lex("［＃中中見出し］\0\0");
        let expected = "\n\n［＃中見出し］\n\n\0\0";
        assert_eq!(serialize_with(&out, opts), expected);

        let mut streamed = String::new();
        serialize_into_with(&out, &mut streamed, opts)
            .expect("serialize into String is infallible");
        assert_eq!(streamed, expected);
    }

    #[test]
    fn serializer_off_preserves_and_canonical_repairs_near_miss() {
        let source = "本文［＃ゴチック］続き［";
        let out = lex(source);

        assert!(requires_verbatim_recovery(&out));
        assert_eq!(serialize_with(&out, SerializeOptions::default()), source);
        assert_eq!(
            serialize_with(
                &out,
                SerializeOptions::default().directives(DirectiveNormalization::Canonical),
            ),
            "本文［＃ゴシック体］続き［"
        );
    }

    #[test]
    fn angle_quote_emitter_and_round_trip_preserve_delimiters() {
        let mut a = Allocator::new();
        let content = a.content_plain("重要");
        let Node::AngleQuote(angle) = a.angle_quote(content) else {
            panic!("angle quote constructor returns its node");
        };
        let store = a.into_store();
        let mut emitted = String::new();
        emit_angle_quote(angle, &store, &mut emitted).expect("serialize into String is infallible");

        assert_eq!(emitted, "≪重要≫");
        assert_eq!(serialize(&lex("前≪重要≫後")), "前≪重要≫後");
    }

    #[test]
    fn self_contained_forward_serializes_bracket_only() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::ast::Node;
        use crate::syntax::{ForwardAttr, ForwardOrigin};

        let mut a = Allocator::new();
        let t = a.content_plain("X");
        let node = a.forward_format(ForwardAttr::Bold, t, ForwardOrigin::SelfContained);
        let Node::Format(f) = node else {
            panic!("forward_format must build a Format node");
        };
        let store = a.into_store();

        let mut s = String::new();
        emit_format(&f, &store, &mut s).expect("serialize into String is infallible");
        assert_eq!(s, "［＃「X」は太字］");
    }

    #[test]
    fn self_contained_heading_serializes_bracket_only() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::ast::Node;
        use crate::syntax::{HeadingKind, HeadingStyle};

        let mut a = Allocator::new();
        let node = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章", true);
        let Node::HeadingHint(h) = node else {
            panic!("heading_hint must build a HeadingHint node");
        };
        let store = a.into_store();

        let mut s = String::new();
        emit_heading_hint(h, &store, &mut s).expect("serialize into String is infallible");
        assert_eq!(s, "［＃「序章」は中見出し］");
    }

    fn emit_via_tracking(node: Node, store: &NodeStore) -> String {
        let mut buf = String::new();
        let mut tw = TrackingWriter::new(&mut buf);
        emit_aozora(
            node,
            store,
            &mut tw,
            EmitContext {
                directives: DirectiveNormalization::Off,
                predecessor: RubyPredecessor::Boundary,
            },
        )
        .expect("serialize into String is infallible");
        buf
    }

    fn on_node_via_sink(store: &NodeStore, kind: SentinelKind, node: NodeRef) -> String {
        let mut buf = String::new();
        let mut tw = TrackingWriter::new(&mut buf);
        let mut sink = SerializeSink {
            store,
            out: &mut tw,
            directives: DirectiveNormalization::Off,
            predecessor: RubyPredecessor::Boundary,
        };
        sink.on_node(kind, node)
            .expect("serialize into String is infallible");
        buf
    }

    fn annotate(raw: &str, kind: DirectiveKind, directives: DirectiveNormalization) -> String {
        let mut a = Allocator::new();
        let d = a.make_directive(raw, kind);
        let store = a.into_store();
        let mut buf = String::new();
        emit_annotation(d, &store, &mut buf, directives)
            .expect("serialize into String is infallible");
        buf
    }

    #[test]
    fn serialize_into_emits_the_walked_source() {
        let out = lex("あ");
        let mut buf = String::new();
        serialize_into(&out, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "あ");
    }

    #[test]
    fn on_node_emits_container_open_and_close() {
        let store = Allocator::new().into_store();
        assert_eq!(
            on_node_via_sink(
                &store,
                SentinelKind::BlockOpen,
                NodeRef::BlockOpen(RegionFormat::Bold { padded: true }),
            ),
            "［＃ここから太字］",
        );
        assert_eq!(
            on_node_via_sink(
                &store,
                SentinelKind::BlockClose,
                NodeRef::BlockClose(RegionClose::Bold { padded: true }),
            ),
            "［＃ここで太字終わり］",
        );
    }

    #[test]
    fn emit_aozora_unit_leaves() {
        let a = Allocator::new();
        let page = a.page_break();
        let body_end = a.body_end();
        let forced = a.forced_break();
        let store = a.into_store();
        assert_eq!(emit_via_tracking(page, &store), "［＃改ページ］");
        assert_eq!(emit_via_tracking(body_end, &store), "［＃本文終わり］");
        assert_eq!(emit_via_tracking(forced, &store), "［＃改行］");
    }

    #[test]
    fn emit_content_one_writes_gaiji_segment() {
        let mut a = Allocator::new();
        let t = a.seg_text("前");
        let g = a.make_gaiji("ほげ", None, false);
        let gseg = a.seg_gaiji(g);
        let c = a.content_segments(&[t, gseg]);
        let store = a.into_store();
        let mut buf = String::new();
        emit_content_one(c, &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "前※［＃「ほげ」］");
    }

    #[test]
    fn emit_content_as_plain_one_writes_every_segment() {
        let mut a = Allocator::new();
        let text = a.seg_text("あ");
        let gaiji = a.make_gaiji("げ", None, false);
        let gseg = a.seg_gaiji(gaiji);
        let directive = a.make_directive("［＃注記］", DirectiveKind::Editorial);
        let dseg = a.seg_annotation(directive);
        let content = a.content_segments(&[text, gseg, dseg]);
        let store = a.into_store();
        let mut buf = String::new();
        emit_content_as_plain_one(content, &store, &mut buf)
            .expect("serialize into String is infallible");
        assert_eq!(buf, "あげ［＃注記］");
    }

    #[test]
    fn ruby_needs_bar_boundaries() {
        let mut a = Allocator::new();
        let uniform = a.content_plain("青梅");
        let g = a.make_gaiji("ほげ", None, false);
        let gseg = a.seg_gaiji(g);
        let all_gaiji = a.content_segments(&[gseg]);
        let t = a.seg_text("前");
        let tail = a.make_gaiji("ふが", None, false);
        let tail_seg = a.seg_gaiji(tail);
        let mixed = a.content_segments(&[t, tail_seg]);
        let resolved = a.make_gaiji("特のへん＋廴＋聿", Some("第3水準1-87-71"), false);
        let resolved_seg = a.seg_gaiji(resolved);
        let resolved_text = a.seg_text("陀多");
        let resolved_mixed = a.content_segments(&[resolved_seg, resolved_text]);
        let store = a.into_store();

        assert!(!ruby_needs_bar(
            &[uniform],
            RubyPredecessor::Boundary,
            &store
        ));
        assert!(!ruby_needs_bar(
            &[all_gaiji],
            RubyPredecessor::Boundary,
            &store
        ));
        assert!(ruby_needs_bar(&[mixed], RubyPredecessor::Boundary, &store));
        assert!(!ruby_needs_bar(
            &[resolved_mixed],
            RubyPredecessor::Boundary,
            &store
        ));
        assert!(ruby_needs_bar(
            &[resolved_mixed],
            RubyPredecessor::Text(RubyBaseClass::Kanji),
            &store
        ));

        assert!(ruby_needs_bar(
            &[uniform],
            RubyPredecessor::Text(RubyBaseClass::Kanji),
            &store
        ));
        assert!(ruby_needs_bar(&[uniform], RubyPredecessor::Bar, &store));
        assert!(!ruby_needs_bar(
            &[uniform],
            RubyPredecessor::Text(RubyBaseClass::Latin),
            &store
        ));
        assert!(ruby_needs_bar(
            &[all_gaiji],
            RubyPredecessor::Gaiji(None),
            &store
        ));
    }

    #[test]
    fn emit_format_bouten_positions() {
        for (position, expected) in [
            (BoutenPosition::Left, "［＃「字」の左に傍点］"),
            (BoutenPosition::Both, "［＃「字」の両側に傍点］"),
            (BoutenPosition::Right, "［＃「字」に傍点］"),
        ] {
            let mut a = Allocator::new();
            let target = a.content_plain("字");
            let Node::Format(f) = a.bouten(
                BoutenKind::Goma,
                target,
                position,
                ForwardOrigin::Referenced,
            ) else {
                panic!("bouten must build a Format node");
            };
            let store = a.into_store();
            let mut buf = String::new();
            emit_format(&f, &store, &mut buf).expect("serialize into String is infallible");
            assert_eq!(buf, expected, "position {position:?}");
        }
    }

    #[test]
    fn emit_format_align_end_offset() {
        for (offset, expected) in [
            (0u8, "［＃「末」は地付き］"),
            (3u8, "［＃「末」は文末より3字上げ揃え］"),
        ] {
            let mut a = Allocator::new();
            let target = a.content_plain("末");
            let Node::Format(f) = a.forward_format(
                ForwardAttr::AlignEnd { offset },
                target,
                ForwardOrigin::Referenced,
            ) else {
                panic!("forward_format must build a Format node");
            };
            let store = a.into_store();
            let mut buf = String::new();
            emit_format(&f, &store, &mut buf).expect("serialize into String is infallible");
            assert_eq!(buf, expected, "offset {offset}");
        }
    }

    #[test]
    fn accent_suffix_exact() {
        assert_eq!(accent_suffix(AccentMark::Acute), "アクサン（´）付き");
        assert_eq!(accent_suffix(AccentMark::Grave), "アクサン（｀）付き");
        assert_eq!(accent_suffix(AccentMark::Umlaut), "ウムラウト（¨）付き");
    }

    #[test]
    fn emit_bouten_targets_plain_run() {
        let mut a = Allocator::new();
        let c = a.content_plain("甲");
        let store = a.into_store();
        let mut buf = String::new();
        emit_bouten_targets(&[c], &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "「甲」");
    }

    #[test]
    fn emit_bouten_targets_splits_segmented_run() {
        let mut a = Allocator::new();
        let t = a.seg_text("甲、乙");
        let g = a.make_gaiji("げ", None, false);
        let gseg = a.seg_gaiji(g);
        let c = a.content_segments(&[t, gseg]);
        let store = a.into_store();
        let mut buf = String::new();
        emit_bouten_targets(&[c], &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "「甲」「乙」");
    }

    #[test]
    fn gaiji_has_mencode_reflects_mencode_presence() {
        assert!(!gaiji_has_mencode(GaijiCanonicalOwned::Unresolved {
            mencode: None
        }));
        assert!(gaiji_has_mencode(GaijiCanonicalOwned::Unicode('あ')));
    }

    #[test]
    fn emit_kaeriten_wraps_mark() {
        let mut a = Allocator::new();
        let Node::Kaeriten(k) = a.kaeriten("レ") else {
            panic!("kaeriten must build a Kaeriten node");
        };
        let store = a.into_store();
        let mut buf = String::new();
        emit_kaeriten(k, &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "［＃レ］");
    }

    #[test]
    fn emit_annotation_directive_normalization() {
        assert_eq!(
            annotate(
                "［＃ゴチック］",
                DirectiveKind::NonCanonical,
                DirectiveNormalization::Canonical,
            ),
            "［＃ゴシック体］",
        );
        assert_eq!(
            annotate(
                "［＃ゴチック］",
                DirectiveKind::Sic,
                DirectiveNormalization::Canonical,
            ),
            "［＃ゴチック］",
        );
        assert_eq!(
            annotate(
                "［＃ゴチック］",
                DirectiveKind::NonCanonical,
                DirectiveNormalization::Off,
            ),
            "［＃ゴチック］",
        );
        assert_eq!(
            annotate(
                "［＃ここから最後まで3字下げ］",
                DirectiveKind::NonCanonical,
                DirectiveNormalization::Degraded,
            ),
            "［＃ここから3字下げ］",
        );
        assert_eq!(
            annotate(
                "［＃ここから最後まで3字下げ］",
                DirectiveKind::NonCanonical,
                DirectiveNormalization::Canonical,
            ),
            "［＃ここから最後まで3字下げ］",
        );
    }

    #[test]
    fn emit_side_note_source() {
        let mut a = Allocator::new();
        let base = a.content_plain("未来");
        let note = a.content_plain("みらい");
        let Node::MarginNote(s) = a.side_note(MarginNoteKind::Gloss, base, note) else {
            panic!("side_note must build a MarginNote node");
        };
        let store = a.into_store();
        let mut buf = String::new();
        emit_side_note(&s, &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "未来［＃「未来」の左に「みらい」の注記］");
    }

    #[test]
    fn emit_sashie_source() {
        let mut a = Allocator::new();
        let Node::Illustration(s) = a.sashie("fig.png", Some("1"), Some("横100×縦200"), None)
        else {
            panic!("sashie must build an Illustration node");
        };
        let store = a.into_store();
        let mut buf = String::new();
        emit_sashie(&s, &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "［＃挿絵1（fig.png、横100×縦200）入る］");
    }

    #[test]
    fn emit_aozora_heading_source() {
        let mut a = Allocator::new();
        let text = a.content_plain("第一章");
        let Node::Heading(h) = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text)
        else {
            panic!("aozora_heading must build a Heading node");
        };
        let store = a.into_store();
        let mut buf = String::new();
        emit_aozora_heading(&h, &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "第一章\n［＃「第一章」は大見出し］");
    }
}
