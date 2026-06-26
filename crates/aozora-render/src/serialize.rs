//! Borrowed-AST Aozora-source serializer.
//!
//! Single forward `match_indices` over the normalized text, dispatch
//! each PUA sentinel through the borrowed registry, bulk-copy plain
//! runs between hits.
//!
//! Round-trip fixed-point pinned by the `serialize_fixed_point`
//! proptest in `tests/serialize_fixed_point.rs`.

use core::fmt::{self, Write};

use crate::walk::{SentinelKind, WalkSink, walk};
use aozora_pipeline::{LexOutput, has_long_rule_line, isolate_decorative_rules};
use aozora_syntax::borrowed::{
    AngleQuote, Content, Directive, ForwardFormat, ForwardOrigin, Gaiji, Heading, HeadingHint,
    Illustration, Kaeriten, MarginNote, Node, NodeRef, Ruby, Segment,
};
use aozora_syntax::{
    BlockStyles, BoutenPosition, ForwardAttr, HeadingKind, HeadingStyle, IndentBlock, IndentLayout,
    LineFormat, RegionClose, RegionFormat, RubySide, SectionKind, is_ruby_base_char,
};

/// Serialize a `LexOutput` back to Aozora source text.
///
/// The output is a fixed point of `serialize ∘ parse` after one
/// pass: a second cycle returns the same bytes. This is the
/// load-bearing corpus-sweep invariant I3, preserved in
/// borrowed form.
///
/// To preserve the fixed point across the parser's
/// [`isolate_decorative_rules`] pre-pass, we run the **same isolator**
/// over the serialized output once before returning. Without that, an
/// inline annotation (e.g. an unmatched ruby trigger `｜...`) sitting
/// directly above a decorative-rule line would emit
/// `…\n----------\n========…` here while the next cycle's sanitize stage
/// would inject a blank to produce `…\n----------\n\n========…`,
/// peeling one extra blank in per round-trip and breaking I3. Running
/// the isolator once at serialize time aligns serialize's output with
/// what the sanitize stage will produce on the next cycle, so a second cycle
/// observes a no-op isolator and the byte sequence converges. The
/// `has_long_rule_line` fast-path keeps the cost O(1) for
/// rule-line-free outputs (the dominant case).
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`Write`] sink.
#[must_use]
pub fn serialize(out: &LexOutput<'_>) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_into(out, &mut s).expect("writing to NewlineCappedWriter never fails");
    let raw = s.into_string();
    if has_long_rule_line(&raw) {
        isolate_decorative_rules(&raw)
    } else {
        raw
    }
}

/// Serialize into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited
/// from the lexer's `Span` width contract; in practice unreachable.
pub fn serialize_into<W: Write>(out: &LexOutput<'_>, writer: &mut W) -> fmt::Result {
    let mut tracking = TrackingWriter::new(writer);
    let mut sink = SerializeSink { out: &mut tracking };
    walk(out, &mut sink)
}

/// The source text of the container **open** marker for `open`.
///
/// `RegionFormat::Indent(2字下げ)` → `［＃ここから2字下げ］`, preserving every
/// payload (N / width / offset / 字組み clause). The inverse of the
/// classifier's open recognition.
///
/// Used by the minimal-diff source splice (#202) to canonicalize a
/// container's open marker. The serialization rule lives here (the single
/// source of truth for marker spelling); the splice layer only calls it.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn container_open_source(open: RegionFormat) -> String {
    let mut s = String::new();
    emit_container_open(open, &mut s).expect("String write is infallible");
    s
}

/// The source text of the container **close** marker that matches `open`.
///
/// `RegionFormat::Indent(2字下げ)` → `［＃ここで字下げ終わり］`; a 字組み compound
/// keeps its width. The close is a pure function of the open
/// ([`RegionClose::of`]).
///
/// Used by the minimal-diff source splice (#202): when a container's family
/// changes, the paired close marker must be rewritten to match the new open,
/// and this derives it without the splice layer re-implementing the spelling.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn container_close_source(open: RegionFormat) -> String {
    let mut s = String::new();
    emit_container_close(RegionClose::of(open), &mut s).expect("String write is infallible");
    s
}

/// Wraps the serialize output and remembers the last `char` emitted.
///
/// `emit_ruby` reads it to decide whether a bare `《reading》` would
/// re-parse to the *same* base (drop `｜`) or a different one (keep `｜`)
/// — ADR 0002. The predecessor may be a preceding NODE (e.g. a kaeriten
/// `二`, which is a ruby-base char) and not just text, so the last char
/// must be tracked at the writer, not per `on_text`.
pub(crate) struct TrackingWriter<W: Write> {
    inner: W,
    last: Option<char>,
}

impl<W: Write> TrackingWriter<W> {
    /// Wrap `inner`, with no predecessor char recorded yet. Shared
    /// construction site for the borrowed and owned serializers.
    pub(crate) const fn new(inner: W) -> Self {
        Self { inner, last: None }
    }

    /// The last `char` written so far, if any. `emit_ruby` reads it to
    /// decide whether a bare `《reading》` drops the explicit `｜` (ADR 0002).
    pub(crate) const fn last(&self) -> Option<char> {
        self.last
    }
}

impl<W: Write> Write for TrackingWriter<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(c) = s.chars().next_back() {
            self.last = Some(c);
        }
        self.inner.write_str(s)
    }
}

/// [`WalkSink`] that re-emits Aozora source text: plain runs are copied
/// verbatim (newlines included — [`Self::WANTS_NEWLINES`] is `false`) and
/// each sentinel is reconstructed through the `emit_*` helpers. The close
/// marker is reconstructed from the close's own [`RegionClose`] (self-
/// sufficient — an unmatched stray close, and a mismatched 傍線終わり closing a
/// 傍点 open, both round-trip byte-exact).
struct SerializeSink<'w, W: Write> {
    out: &'w mut TrackingWriter<W>,
}

impl<W: Write> WalkSink for SerializeSink<'_, W> {
    // Serialization copies `\n` verbatim, so it is not a structural event.
    const WANTS_NEWLINES: bool = false;

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef<'_>) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(node))
            | (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(node)) => emit_aozora(node, self.out),
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                emit_container_open(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                emit_container_close(close, self.out)
            }
            // Sentinel hit without a corresponding registry entry, or a
            // kind/variant mismatch — best-effort skip (the per-table
            // lookups silently dropped these too).
            _ => Ok(()),
        }
    }
}

fn emit_aozora<W: Write>(node: Node<'_>, out: &mut TrackingWriter<W>) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_ruby(r, out),
        Node::Format(f) => emit_format(f, out),
        Node::Gaiji(g) => emit_gaiji(g, out),
        Node::Kaeriten(k) => emit_kaeriten(k, out),
        Node::Directive(a) => emit_annotation(a, out),
        Node::AngleQuote(d) => emit_angle_quote(d, out),
        Node::MarginNote(s) => emit_side_note(s, out),
        Node::PageBreak => out.write_str("［＃改ページ］"),
        Node::BodyEnd => out.write_str("［＃本文終わり］"),
        Node::ForcedBreak => out.write_str("［＃改行］"),
        Node::SectionBreak(kind) => emit_section_break(kind, out),
        Node::Line(lf) => emit_line(lf, out),
        Node::Illustration(s) => emit_sashie(s, out),
        Node::HeadingHint(h) => emit_heading_hint(h, out),
        Node::Heading(h) => emit_aozora_heading(h, out),
        // Variants the serializer doesn't yet cover: Container is
        // routed through the open/close sentinel path; Warichu /
        // Framed / Heading land here as a diagnostic
        // placeholder, matching the legacy serializer's behavior.
        _ => {
            out.write_str("<!-- unsupported-aozora: ")?;
            out.write_str(node.xml_node_name())?;
            out.write_str(" -->")
        }
    }
}

fn emit_ruby<W: Write>(r: &Ruby<'_>, out: &mut TrackingWriter<W>) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        // Left-side ruby: reconstruct `base［＃「base」の左に「reading」のルビ］`.
        // The base is the pulled-back predecessor, so it precedes the directive.
        emit_content(r.base.get(), out)?;
        out.write_str("［＃「")?;
        emit_content(r.base.get(), out)?;
        out.write_str("」の左に「")?;
        emit_content(r.reading.get(), out)?;
        return out.write_str("」のルビ］");
    }
    // Canonical right-side ruby is BARE `base《reading》` (ADR 0003); the
    // explicit base marker `｜` is emitted only when omitting it would let
    // the implicit-base scan re-parse a *different* base (ADR 0002). Read
    // the preceding char BEFORE writing anything.
    if ruby_needs_bar(r.base.get(), out.last) {
        out.write_char('｜')?;
    }
    emit_content(r.base.get(), out)?;
    out.write_char('《')?;
    emit_content(r.reading.get(), out)?;
    out.write_char('》')
}

/// True when a bare `base《reading》` would NOT re-parse to `base` as its
/// implicit base, so the explicit `｜` must be kept (ADR 0002). Cases:
/// (a) `base` carries a non-base char — the implicit scan (a trailing
/// run of [`is_ruby_base_char`]) cannot select this exact run; (b) the
/// char immediately before `base` is itself a base char — the implicit
/// scan would greedily extend leftward past `base`; (c) the char
/// immediately before `base` is a `｜` — dropping our own `｜` would let
/// that preceding bar become the base marker on re-parse, shedding one
/// bar per round-trip (non-idempotent). A non-`Plain` base (defensive;
/// right-side bases are always `Plain`) always keeps `｜`.
fn ruby_needs_bar(base: Content<'_>, prev: Option<char>) -> bool {
    base.as_plain().is_none_or(|s| {
        s.chars().any(|c| !is_ruby_base_char(c))
            || prev.is_some_and(|c| is_ruby_base_char(c) || c == '｜')
    })
}

fn emit_side_note<W: Write>(s: &MarginNote<'_>, out: &mut W) -> fmt::Result {
    // Reconstruct `base［＃「base{connector}note{suffix}`; the base is the
    // pulled-back predecessor, so it precedes the directive (mirrors the
    // left-side ruby round-trip in `emit_ruby`). The connector + keyword
    // depend on the flavour (注記 vs 傍記) — see `MarginNoteKind::serialize_affixes`.
    let (connector, suffix) = s.kind.serialize_affixes();
    emit_content(s.base.get(), out)?;
    out.write_str("［＃「")?;
    emit_content(s.base.get(), out)?;
    out.write_str(connector)?;
    emit_content(s.note.get(), out)?;
    out.write_str(suffix)
}

/// Re-emit a forward-reference leaf (`<target>［＃「<target>」は…／に…］`).
///
/// A [`Reclaimed`](ForwardOrigin::Reclaimed) origin drives the leading-literal
/// re-emit that holds the parse∘serialize fixed point. The attribute selects
/// the shape: 傍点 / 傍線 take the multi-`「」` target + `に` / `の左に` connector;
/// every other forward attribute takes the `「target」は<keyword>` shape (with
/// the magnitude spelled out for 文字サイズ).
fn emit_format<W: Write>(f: &ForwardFormat<'_>, out: &mut W) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Reclaimed) {
        emit_content_as_plain(f.target.get(), out)?;
    }
    if let ForwardAttr::Bouten { kind, position } = f.attr {
        out.write_str("［＃")?;
        emit_bouten_targets(f.target.get(), out)?;
        match position {
            BoutenPosition::Left => out.write_str("の左に")?,
            _ => out.write_char('に')?,
        }
        out.write_str(kind.keyword())?;
        return out.write_char('］');
    }
    out.write_str("［＃「")?;
    emit_content_as_plain(f.target.get(), out)?;
    out.write_str("」は")?;
    // 文字サイズ carries a magnitude the static keyword table can't hold.
    if let ForwardAttr::FontSize(shift) = f.attr {
        let word = if shift.larger() {
            "大きな"
        } else {
            "小さな"
        };
        write!(out, "{}段階{word}文字", shift.magnitude())?;
    } else {
        out.write_str(f.attr.keyword())?;
    }
    out.write_char('］')
}

fn emit_bouten_targets<W: Write>(c: Content<'_>, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(s) => {
            out.write_char('「')?;
            out.write_str(s)?;
            out.write_char('」')
        }
        Content::Segments(segs) => {
            let mut any = false;
            for seg in segs.iter().copied() {
                if let Segment::Text(t) = seg
                    && !t.is_empty()
                {
                    for part in t.split('、').filter(|p| !p.is_empty()) {
                        out.write_char('「')?;
                        out.write_str(part)?;
                        out.write_char('」')?;
                        any = true;
                    }
                }
            }
            if !any {
                out.write_char('「')?;
                out.write_char('」')?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn emit_gaiji<W: Write>(g: &Gaiji<'_>, out: &mut W) -> fmt::Result {
    // Standalone (#122) gaiji had no leading `※` in the source; omit it
    // so `serialize ∘ parse` stays a fixed point.
    if !g.standalone {
        out.write_char('※')?;
    }
    out.write_str("［＃")?;
    // The composed-glyph form (`「X」の「Y」に代えて「Z」`) is captured verbatim,
    // already carrying its own `「」` structure, so emit it raw — wrapping it in
    // another `「…」` would double-quote and break the round-trip. The simple
    // form's description is bare text and gets the `「…」` wrapper.
    if g.hint.contains(['「', '」']) {
        out.write_str(g.hint)?;
    } else {
        out.write_char('「')?;
        out.write_str(g.hint)?;
        out.write_char('」')?;
    }
    if g.canonical.has_mencode() {
        out.write_char('、')?;
        g.canonical.write_mencode(out)?;
    }
    out.write_char('］')
}

fn emit_kaeriten<W: Write>(k: &Kaeriten<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(k.mark.as_str())?;
    out.write_char('］')
}

fn emit_annotation<W: Write>(a: &Directive<'_>, out: &mut W) -> fmt::Result {
    out.write_str(a.raw.as_str())
}

fn emit_angle_quote<W: Write>(d: &AngleQuote<'_>, out: &mut W) -> fmt::Result {
    out.write_char('≪')?;
    emit_content(d.content.get(), out)?;
    out.write_char('≫')
}

pub(crate) fn emit_section_break<W: Write>(kind: SectionKind, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(kind.keyword())?;
    out.write_char('］')
}

pub(crate) fn emit_line<W: Write>(lf: LineFormat, out: &mut W) -> fmt::Result {
    match lf {
        LineFormat::Indent { amount: 1 } => out.write_str("［＃字下げ］"),
        LineFormat::Indent { amount } => write!(out, "［＃{amount}字下げ］"),
        LineFormat::AlignEnd { offset: 0 } => out.write_str("［＃地付き］"),
        LineFormat::AlignEnd { offset } => write!(out, "［＃地から{offset}字上げ］"),
        LineFormat::Center { page: true } => out.write_str("［＃ページの左右中央］"),
        LineFormat::Center { page: false } => out.write_str("［＃中央揃え］"),
        LineFormat::Framed => out.write_str("［＃罫囲み］"),
        // `LineFormat` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

fn emit_sashie<W: Write>(s: &Illustration<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    if let Some(description) = s.description {
        // General image form `<説明>（file）入る` — the leading text is the
        // alt; there is no 挿絵 keyword / number / trailing 「caption」.
        out.write_str(description)?;
    } else {
        out.write_str("挿絵")?;
        if let Some(number) = s.number {
            out.write_str(number.as_str())?;
        }
    }
    out.write_char('（')?;
    out.write_str(s.file.as_str())?;
    if let Some(dims) = s.dimensions {
        // Reconstruct `（file、横W×縦H）` so the pixel-size note round-trips
        // (it rides in `dimensions`, out of the clean `file` path).
        out.write_char('、')?;
        out.write_str(dims)?;
    }
    out.write_char('）')?;
    if let Some(caption) = s.caption {
        out.write_char('「')?;
        emit_content_as_plain(caption, out)?;
        out.write_char('」')?;
    }
    out.write_str("入る］")
}

/// The optional `同行` / `窓` style prefix that precedes the level keyword in
/// a `…は<style><level>見出し` directive (empty for the standard style).
pub(crate) const fn heading_style_keyword(style: HeadingStyle) -> &'static str {
    match style {
        HeadingStyle::SameLine => "同行",
        HeadingStyle::Window => "窓",
        // Standard and any future style serialize without a prefix.
        _ => "",
    }
}

/// The `大 / 中 / 小見出し` level keyword (no delimiter), shared by the leaf
/// heading, the hint, and the paired / block [`RegionFormat::Heading`].
pub(crate) const fn heading_level_word(kind: HeadingKind) -> &'static str {
    match kind {
        HeadingKind::Medium => "中見出し",
        HeadingKind::Small => "小見出し",
        // 大見出し and any future level fall back to the 大見出し form.
        _ => "大見出し",
    }
}

fn emit_aozora_heading<W: Write>(h: &Heading<'_>, out: &mut W) -> fmt::Result {
    // Reconstruct the promoted forward-reference shape, byte-identical to
    // the source the classifier consumed:
    //   <text>\n［＃「<text>」は<同行|窓>?<大|中|小>見出し］
    emit_content(h.text.get(), out)?;
    out.write_str("\n［＃「")?;
    emit_content(h.text.get(), out)?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.kind))?;
    out.write_str("］")
}

fn emit_heading_hint<W: Write>(h: &HeadingHint<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(h.target.as_str())?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.level))?;
    out.write_str("］")
}

/// `左に` left-side prefix for a bouten range marker, or `""`.
const fn bouten_left_prefix(position: BoutenPosition) -> &'static str {
    match position {
        BoutenPosition::Left => "左に",
        _ => "",
    }
}

/// Serialize a container open marker from its [`RegionFormat`]. 傍点 / 傍線
/// ranges reconstruct `［＃<左に?><variant>］`; every other family spells its
/// own opener (preserving every payload — dropping N / width / offset / the
/// 字組み clause would be a §7.6 fixed-point violation).
pub(crate) fn emit_container_open<W: Write>(open: RegionFormat, out: &mut W) -> fmt::Result {
    match open {
        RegionFormat::Bouten { kind, position } => write!(
            out,
            "［＃{}{}］",
            bouten_left_prefix(position),
            kind.keyword()
        ),
        RegionFormat::Indent(block) => emit_indent_open(block, out),
        RegionFormat::Bold { padded: false } => out.write_str("［＃太字］"),
        RegionFormat::Bold { padded: true } => out.write_str("［＃ここから太字］"),
        RegionFormat::Italic { padded: false } => out.write_str("［＃斜体］"),
        RegionFormat::Italic { padded: true } => out.write_str("［＃ここから斜体］"),
        RegionFormat::AlignEnd { offset: 0 } => out.write_str("［＃ここから地付き］"),
        RegionFormat::AlignEnd { offset } => write!(out, "［＃ここから地から{offset}字上げ］"),
        RegionFormat::LineWidth(width) => write!(out, "［＃ここから{}字詰め］", width.0),
        RegionFormat::Heading {
            level,
            style,
            padded,
        } => write!(
            out,
            "［＃{}{}{}］",
            if padded { "ここから" } else { "" },
            heading_style_keyword(style),
            heading_level_word(level),
        ),
        RegionFormat::Columns(count) => write!(out, "［＃ここから{}段組み］", count.0),
        RegionFormat::Table => out.write_str("［＃ここから表］"),
        RegionFormat::Horizontal => out.write_str("［＃ここから横組み］"),
        RegionFormat::FontSize(shift) => {
            let word = if shift.larger() {
                "大きな"
            } else {
                "小さな"
            };
            write!(out, "［＃ここから{}段階{word}文字］", shift.magnitude())
        }
        RegionFormat::SmallScript(side) => {
            write!(out, "［＃行{}小書き］", small_script_side_word(side))
        }
        RegionFormat::Caption { padded: true } => out.write_str("［＃ここからキャプション］"),
        RegionFormat::Caption { padded: false } => out.write_str("［＃キャプション］"),
        // `Warichu` is the block 割り注 region (the inline ［＃割り注］ is an
        // `Directive{WarichuOpen}`), so it serializes to the ここから form.
        RegionFormat::Warichu => out.write_str("［＃ここから割り注］"),
        RegionFormat::Framed => out.write_str("［＃罫囲み］"),
        RegionFormat::CombineUpright => out.write_str("［＃縦中横］"),
        // `RegionFormat` is `#[non_exhaustive]`; a future family falls back to
        // the most common opener until it is given a spelling here.
        _ => out.write_str("［＃ここから字下げ］"),
    }
}

/// Serialize a `［＃ここから…字下げ…］` opener from its [`IndentBlock`] (#78).
///
/// Built incrementally in a fixed **canonical clause order** (wrap → center →
/// line-layout → bold → horizontal → framed → font), independent of the source
/// order, so the compound is a 1-pass serialize fixed point. The order and
/// keywords mirror `render_container_open` and [`BlockStyles::iter_formats`].
/// The `..`-free destructure means a new [`IndentBlock`] / [`BlockStyles`]
/// field is compiler-flagged here rather than silently dropped from the marker
/// (the §7.6 param-drop bug class).
fn emit_indent_open<W: Write>(block: IndentBlock, out: &mut W) -> fmt::Result {
    let IndentBlock {
        amount,
        wrap,
        center,
        layout,
        styles,
    } = block;
    let BlockStyles {
        bold,
        horizontal,
        framed,
        font,
    } = styles;

    // The idiomatic no-number `［＃ここから字下げ］` form is reserved for a bare
    // single-char indent with no clauses; anything else takes the numbered form.
    let bare = wrap.is_none()
        && !center
        && matches!(layout, IndentLayout::None)
        && !bold
        && !horizontal
        && !framed
        && font.is_none();
    if amount == 1 && bare {
        return out.write_str("［＃ここから字下げ］");
    }

    write!(out, "［＃ここから{amount}字下げ")?;
    if let Some(wrap) = wrap {
        write!(out, "、折り返して{wrap}字下げ")?;
    }
    if center {
        out.write_str("、ページの左右中央に")?;
    }
    match layout {
        IndentLayout::Kumi(kumi) => write!(out, "、{}行{}字組みで", kumi.lines, kumi.width)?,
        IndentLayout::LineWidth(width) => write!(out, "、{}字詰め", width.0)?,
        IndentLayout::None => {}
    }
    if bold {
        out.write_str("、ゴシック体")?;
    }
    if horizontal {
        out.write_str("、横書き")?;
    }
    if framed {
        out.write_str("、罫囲み")?;
    }
    if let Some(shift) = font {
        // `小さい活字` is the canonical one-stage-smaller spelling (the only
        // font compound the corpus attests); a general magnitude falls back to
        // the `N段階…文字` form so the field stays round-trippable.
        if shift.0.get() == -1 {
            out.write_str("、小さい活字")?;
        } else if shift.larger() {
            write!(out, "、{}段階大きな文字", shift.magnitude())?;
        } else {
            write!(out, "、{}段階小さな文字", shift.magnitude())?;
        }
    }
    out.write_str("］")
}

/// 小書き side keyword: `右` / `左`.
const fn small_script_side_word(side: BoutenPosition) -> &'static str {
    match side {
        BoutenPosition::Left => "左",
        _ => "右",
    }
}

/// Serialize a container close marker from the close's own [`RegionClose`].
///
/// Self-sufficient: the 字組み close keeps its own width, the 太字/斜体
/// block-vs-inline form its own `padded`, the 傍点/傍線 close its own family
/// (so a mismatched `［＃傍線終わり］` closing a `［＃傍点］` round-trips), and
/// a stray close with no matching open still emits its marker.
pub(crate) fn emit_container_close<W: Write>(close: RegionClose, out: &mut W) -> fmt::Result {
    match close {
        RegionClose::Bouten { kind, position } => write!(
            out,
            "［＃{}{}終わり］",
            bouten_left_prefix(position),
            kind.keyword()
        ),
        RegionClose::Bold { padded: false } => out.write_str("［＃太字終わり］"),
        RegionClose::Bold { padded: true } => out.write_str("［＃ここで太字終わり］"),
        RegionClose::Italic { padded: false } => out.write_str("［＃斜体終わり］"),
        RegionClose::Italic { padded: true } => out.write_str("［＃ここで斜体終わり］"),
        // #78 字組み compound — the close keeps its own width so the marker
        // round-trips byte-exact; every other indent close is the generic form.
        RegionClose::Indent {
            kumi_width: Some(width),
        } => write!(out, "［＃ここで字下げ、{}字組み終わり］", width.0),
        RegionClose::LineWidth => out.write_str("［＃ここで字詰め終わり］"),
        RegionClose::Heading {
            level,
            style,
            padded,
        } => write!(
            out,
            "［＃{}{}{}終わり］",
            if padded { "ここで" } else { "" },
            heading_style_keyword(style),
            heading_level_word(level),
        ),
        RegionClose::Columns => out.write_str("［＃ここで段組み終わり］"),
        RegionClose::Table => out.write_str("［＃ここで表終わり］"),
        RegionClose::Horizontal => out.write_str("［＃ここで横組み終わり］"),
        RegionClose::FontSize { larger: true } => out.write_str("［＃ここで大きな文字終わり］"),
        RegionClose::FontSize { larger: false } => out.write_str("［＃ここで小さな文字終わり］"),
        RegionClose::SmallScript(side) => {
            write!(out, "［＃行{}小書き終わり］", small_script_side_word(side))
        }
        RegionClose::Caption { padded: true } => out.write_str("［＃ここでキャプション終わり］"),
        RegionClose::Caption { padded: false } => out.write_str("［＃キャプション終わり］"),
        RegionClose::Warichu => out.write_str("［＃ここで割り注終わり］"),
        RegionClose::Framed => out.write_str("［＃罫囲み終わり］"),
        RegionClose::AlignEnd => out.write_str("［＃ここで地付き終わり］"),
        RegionClose::CombineUpright => out.write_str("［＃縦中横終わり］"),
        // The generic `字下げ終わり` — the `Indent { kumi_width: None }` close
        // (plain / 字詰め / 折り返して / 中央 indents) and the `#[non_exhaustive]`
        // forward-compat fallback.
        _ => out.write_str("［＃ここで字下げ終わり］"),
    }
}

fn emit_content<W: Write>(c: Content<'_>, out: &mut W) -> fmt::Result {
    for seg in c {
        match seg {
            Segment::Text(t) => out.write_str(t)?,
            Segment::Gaiji(g) => emit_gaiji(g, out)?,
            Segment::Directive(a) => emit_annotation(a, out)?,
            _ => {}
        }
    }
    Ok(())
}

fn emit_content_as_plain<W: Write>(c: Content<'_>, out: &mut W) -> fmt::Result {
    for seg in c {
        match seg {
            Segment::Text(t) => out.write_str(t)?,
            Segment::Gaiji(g) => out.write_str(g.hint)?,
            Segment::Directive(a) => out.write_str(a.raw.as_str())?,
            _ => {}
        }
    }
    Ok(())
}

/// Output buffer that caps consecutive `\n` runs at two on-the-fly.
///
/// The classify stage pads every block sentinel with `\n\n`
/// unconditionally, so naively round-tripping the serializer's
/// output back through parse inflates the blank-line run by two
/// per iteration. Capping at 2 here makes `serialize ∘ parse` a
/// fixed point after the first pass.
pub(crate) struct NewlineCappedWriter {
    out: String,
    trailing_newlines: usize,
}

impl NewlineCappedWriter {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            out: String::with_capacity(cap),
            trailing_newlines: 0,
        }
    }

    fn push_str_internal(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if !s.contains('\n') {
            self.out.push_str(s);
            self.trailing_newlines = 0;
            return;
        }
        let mut cursor = 0;
        for (nl_pos, _) in s.match_indices('\n') {
            if nl_pos > cursor {
                self.out.push_str(&s[cursor..nl_pos]);
                self.trailing_newlines = 0;
            }
            self.trailing_newlines += 1;
            if self.trailing_newlines <= 2 {
                self.out.push('\n');
            }
            cursor = nl_pos + 1;
        }
        if cursor < s.len() {
            self.out.push_str(&s[cursor..]);
            self.trailing_newlines = 0;
        }
    }

    pub(crate) fn into_string(self) -> String {
        self.out
    }
}

impl Write for NewlineCappedWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str_internal(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        if c == '\n' {
            self.trailing_newlines += 1;
            if self.trailing_newlines <= 2 {
                self.out.push('\n');
            }
        } else {
            self.trailing_newlines = 0;
            self.out.push(c);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;

    fn ser(src: &str) -> String {
        let arena = Arena::new();
        let out = aozora_pipeline::lex(src, &arena);
        serialize(&out)
    }

    #[test]
    fn plain_text_round_trips() {
        assert_eq!(ser("hello world"), "hello world");
    }

    #[test]
    fn explicit_ruby_round_trips() {
        // Canonical bare form (the redundant `｜` is dropped).
        let out = ser("｜青梅《おうめ》");
        assert!(
            out.contains("青梅《おうめ》") && !out.contains('｜'),
            "got {out:?}"
        );
    }

    #[test]
    fn page_break_round_trips() {
        let out = ser("text［＃改ページ］more");
        assert!(out.contains("［＃改ページ］"));
    }

    #[test]
    fn paired_container_round_trips() {
        let out = ser("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
        assert!(out.contains("［＃ここから"));
        assert!(out.contains("［＃ここで"));
    }

    #[test]
    fn serialize_is_a_fixed_point_after_one_pass() {
        let inputs = [
            "hello",
            "｜青梅《おうめ》",
            "text［＃改ページ］more",
            "※［＃「木＋吶のつくり」、第3水準1-85-54］",
            "［＃ここから2字下げ］\nA\n［＃ここで字下げ終わり］",
            "本文［＃太字］註［＃太字終わり］。",
            "重要［＃「重要」は太字］な点。",
            "［＃ここから斜体］\nA\n［＃ここで斜体終わり］",
            // #78 compound indent: styles, 4-way, source-order normalisation,
            // the explicit compound closer folding to the generic 字下げ終わり,
            // and the unknown-clause decline (stays verbatim).
            "［＃ここから2字下げ、小さい活字］\nA\n［＃ここで字下げ終わり、小さい活字も終わり］",
            "［＃ここから4字下げ、横書き、中央揃え、罫囲み］\nA\n［＃ここで字下げ終わり］",
            "［＃ここから3字下げ、ゴシック体］\nA\n［＃ここで字下げ終わり］",
            "［＃ここから天付き、折り返して1字下げ］\nA\n［＃ここで字下げ終わり］",
            "［＃ここから5字下げ、本文よりひとまわり大きい太ゴシック体］\nA\n［＃ここで字下げ終わり］",
            // #78 structural leaf markers.
            "行頭［＃改行］行末",
            "本編［＃本文終わり］",
        ];
        for src in inputs {
            let first = ser(src);
            let second = ser(&first);
            assert_eq!(first, second, "fixed point broken for {src:?}");
        }
    }

    // -------------------------------------------------------------------
    // Per-construct exact round-trip. Each input lexes into one node /
    // container kind and serializes back to the canonical fixed-point
    // source. The exact `assert_eq!` pins every `emit_*` arm.
    //
    // Block sentinels carry `\n\n` padding from the classify stage, capped at two by
    // the NewlineCappedWriter, so a standalone block node serializes
    // wrapped in `\n\n…\n\n`. Inline / block-leaf-without-padding nodes
    // serialize bare.
    // -------------------------------------------------------------------

    // --- Ruby (right + left) + side note -------------------------------

    #[test]
    fn redundant_bar_is_dropped_to_canonical_bare() {
        // The `｜` before an all-kanji base at line start is redundant — a
        // bare `青梅《おうめ》` re-parses to the same base — so the canonical
        // form (ADR 0002/0003) drops it.
        assert_eq!(ser("｜青梅《おうめ》"), "青梅《おうめ》");
    }

    #[test]
    fn implicit_ruby_stays_bare() {
        // Already canonical; no `｜` is introduced.
        assert_eq!(ser("青梅《おうめ》"), "青梅《おうめ》");
    }

    #[test]
    fn bar_is_kept_when_base_char_precedes_base() {
        // `頃` is a kanji (ruby-base char); without `｜` the implicit scan
        // would extend the base to `頃青梅`, so the `｜` is mandatory.
        assert_eq!(ser("頃｜青梅《おうめ》"), "頃｜青梅《おうめ》");
    }

    #[test]
    fn bar_is_kept_when_base_mixes_classes() {
        // A base with a non-base char (kana `め`) cannot be re-derived by
        // the trailing-kanji scan, so `｜` stays.
        assert_eq!(ser("｜お目《おめ》"), "｜お目《おめ》");
    }

    #[test]
    fn bar_is_kept_after_a_preceding_bar_so_count_is_stable() {
        // A `｜` immediately before the base would become the marker on
        // re-parse if we dropped our own; keeping it makes the bar count a
        // fixed point (regression: `｜｜｜｜青空…` shed one bar per pass).
        assert_eq!(ser("｜｜｜｜青空《あおぞら》"), "｜｜｜｜青空《あおぞら》");
        assert_eq!(ser("｜｜青空《あおぞら》"), "｜｜青空《あおぞら》");
    }

    #[test]
    fn left_ruby_reconstructs_forward_directive() {
        assert_eq!(
            ser("再読［＃「再読」の左に「さい」のルビ］"),
            "再読［＃「再読」の左に「さい」のルビ］"
        );
    }

    #[test]
    fn side_note_reconstructs_forward_directive() {
        assert_eq!(
            ser("底本「青空」［＃「青空」の左に「注記」の注記］"),
            "底本「青空」青空［＃「青空」の左に「注記」の注記］"
        );
    }

    #[test]
    fn boki_reconstructs_forward_directive() {
        // 傍記 keeps the bare `に` (no 左) and its own keyword; the target
        // is the immediate predecessor, so the round-trip is byte-identical.
        assert_eq!(
            ser("資本主義の一般的危機［＃「危機」に「×」の傍記］"),
            "資本主義の一般的危機［＃「危機」に「×」の傍記］"
        );
    }

    // --- Bouten leaf (forward reference) -------------------------------

    #[test]
    fn bouten_goma_default_keyword() {
        assert_eq!(
            ser("可哀想［＃「可哀想」に傍点］"),
            "可哀想［＃「可哀想」に傍点］"
        );
    }

    #[test]
    fn bouten_named_kinds_keep_keyword() {
        for (src, want) in [
            ("X［＃「X」に白ゴマ傍点］", "X［＃「X」に白ゴマ傍点］"),
            ("X［＃「X」に丸傍点］", "X［＃「X」に丸傍点］"),
            ("X［＃「X」に二重丸傍点］", "X［＃「X」に二重丸傍点］"),
            ("X［＃「X」に蛇の目傍点］", "X［＃「X」に蛇の目傍点］"),
            ("X［＃「X」に傍線］", "X［＃「X」に傍線］"),
            ("X［＃「X」に波線］", "X［＃「X」に波線］"),
            ("X［＃「X」に二重傍線］", "X［＃「X」に二重傍線］"),
            ("X［＃「X」に鎖線］", "X［＃「X」に鎖線］"),
            ("X［＃「X」に破線］", "X［＃「X」に破線］"),
            ("X［＃「X」に黒三角傍点］", "X［＃「X」に黒三角傍点］"),
        ] {
            assert_eq!(ser(src), want, "bouten kind round-trip for {src:?}");
        }
    }

    #[test]
    fn bouten_left_position_emits_no_hidari_prefix_form() {
        assert_eq!(ser("X［＃「X」の左に傍点］"), "X［＃「X」の左に傍点］");
    }

    // --- 縦中横 leaf ---------------------------------------------------

    #[test]
    fn tate_chu_yoko_leaf_round_trips() {
        assert_eq!(ser("12［＃「12」は縦中横］"), "12［＃「12」は縦中横］");
    }

    // --- Emphasis leaf (every kind, incl. FontSize ±) ------------------

    #[test]
    fn emphasis_bold_leaf_round_trips() {
        assert_eq!(ser("重要［＃「重要」は太字］"), "重要［＃「重要」は太字］");
    }

    #[test]
    fn emphasis_italic_leaf_round_trips() {
        assert_eq!(ser("X［＃「X」は斜体］"), "X［＃「X」は斜体］");
    }

    #[test]
    fn emphasis_super_and_sub_script_round_trip() {
        assert_eq!(
            ser("X［＃「X」は上付き小文字］"),
            "X［＃「X」は上付き小文字］"
        );
        assert_eq!(
            ser("X［＃「X」は下付き小文字］"),
            "X［＃「X」は下付き小文字］"
        );
    }

    #[test]
    fn emphasis_small_script_round_trips() {
        assert_eq!(ser("X［＃「X」は行右小書き］"), "X［＃「X」は行右小書き］");
        assert_eq!(ser("X［＃「X」は行左小書き］"), "X［＃「X」は行左小書き］");
    }

    #[test]
    fn emphasis_font_size_positive_emits_bigger_word() {
        assert_eq!(
            ser("X［＃「X」は3段階大きな文字］"),
            "X［＃「X」は3段階大きな文字］"
        );
    }

    #[test]
    fn emphasis_font_size_negative_emits_smaller_word() {
        assert_eq!(
            ser("X［＃「X」は2段階小さな文字］"),
            "X［＃「X」は2段階小さな文字］"
        );
    }

    // --- Gaiji ---------------------------------------------------------

    #[test]
    fn gaiji_simple_description_wraps_in_quotes() {
        // The description is bare text → it gets the 「…」 wrapper back.
        assert_eq!(
            ser("※［＃「○○」、第3水準1-85-54］"),
            "※［＃「○○」、第3水準1-85-54］"
        );
    }

    // --- Kaeriten ------------------------------------------------------

    #[test]
    fn kaeriten_round_trips() {
        assert_eq!(ser("一二［＃レ］"), "一二［＃レ］");
    }

    // --- Directive (unknown directive flows through raw) ---------------

    #[test]
    fn unknown_annotation_round_trips_raw() {
        // `［＃字下げ］` (no number) is an unknown annotation, re-emitted raw.
        assert_eq!(ser("［＃字下げ］"), "［＃字下げ］");
    }

    // --- AngleQuote ----------------------------------------------------

    #[test]
    fn angle_quote_round_trips() {
        assert_eq!(ser("≪重要≫"), "≪重要≫");
    }

    // --- PageBreak + SectionBreak (block leaves, padded) ---------------

    #[test]
    fn page_break_block_leaf_padded() {
        assert_eq!(ser("［＃改ページ］"), "\n\n［＃改ページ］\n\n");
    }

    #[test]
    fn section_break_kinds_round_trip_padded() {
        assert_eq!(ser("［＃改丁］"), "\n\n［＃改丁］\n\n");
        assert_eq!(ser("［＃改段］"), "\n\n［＃改段］\n\n");
        assert_eq!(ser("［＃改見開き］"), "\n\n［＃改見開き］\n\n");
    }

    // --- Indent leaf + AlignEnd leaf -----------------------------------

    #[test]
    fn indent_leaf_keeps_amount() {
        assert_eq!(ser("［＃2字下げ］"), "［＃2字下げ］");
    }

    #[test]
    fn align_end_leaf_zero_offset_is_jizuki() {
        assert_eq!(ser("［＃地付き］"), "［＃地付き］");
    }

    #[test]
    fn align_end_leaf_nonzero_offset_keeps_number() {
        assert_eq!(ser("［＃地から2字上げ］"), "［＃地から2字上げ］");
    }

    // --- Center leaf (page vs line) ------------------------------------

    #[test]
    fn center_page_and_line_round_trip() {
        assert_eq!(ser("［＃ページの左右中央］"), "［＃ページの左右中央］");
        assert_eq!(ser("［＃中央揃え］"), "［＃中央揃え］");
    }

    // --- Illustration (keyword form + dimensions) ----------------------------

    #[test]
    fn sashie_keyword_form_padded() {
        assert_eq!(
            ser("［＃挿絵（fig.png）入る］"),
            "\n\n［＃挿絵（fig.png）入る］\n\n"
        );
    }

    #[test]
    fn sashie_dimensions_round_trip() {
        assert_eq!(
            ser("［＃挿絵（fig.png、横480×縦640）入る］"),
            "\n\n［＃挿絵（fig.png、横480×縦640）入る］\n\n"
        );
    }

    // --- Heading leaf (every level × style) ----------------------

    #[test]
    fn aozora_heading_levels_round_trip() {
        assert_eq!(
            ser("見出し\n［＃「見出し」は大見出し］"),
            "\n\n見出し\n［＃「見出し」は大見出し］\n\n"
        );
        assert_eq!(
            ser("見出し\n［＃「見出し」は中見出し］"),
            "\n\n見出し\n［＃「見出し」は中見出し］\n\n"
        );
        assert_eq!(
            ser("見出し\n［＃「見出し」は小見出し］"),
            "\n\n見出し\n［＃「見出し」は小見出し］\n\n"
        );
    }

    #[test]
    fn aozora_heading_styles_round_trip() {
        assert_eq!(
            ser("見出し\n［＃「見出し」は窓中見出し］"),
            "\n\n見出し\n［＃「見出し」は窓中見出し］\n\n"
        );
        assert_eq!(
            ser("見出し\n［＃「見出し」は同行小見出し］"),
            "\n\n見出し\n［＃「見出し」は同行小見出し］\n\n"
        );
    }

    // -------------------------------------------------------------------
    // Container open / close — one per ContainerKind family + payload.
    // -------------------------------------------------------------------

    #[test]
    fn indent_block_amount_one_uses_no_number() {
        assert_eq!(
            ser("［＃ここから字下げ］\nA\n［＃ここで字下げ終わり］"),
            "\n\n［＃ここから字下げ］\n\nA\n\n［＃ここで字下げ終わり］\n\n"
        );
    }

    #[test]
    fn indent_block_keeps_amount() {
        assert_eq!(
            ser("［＃ここから2字下げ］\nA\n［＃ここで字下げ終わり］"),
            "\n\n［＃ここから2字下げ］\n\nA\n\n［＃ここで字下げ終わり］\n\n"
        );
    }

    #[test]
    fn indent_block_wrap_form_keeps_both_amounts() {
        assert_eq!(
            ser("［＃ここから3字下げ、折り返して5字下げ］\nA\n［＃ここで字下げ終わり］"),
            "\n\n［＃ここから3字下げ、折り返して5字下げ］\n\nA\n\n［＃ここで字下げ終わり］\n\n"
        );
    }

    #[test]
    fn indent_block_center_form_keeps_page_center() {
        assert_eq!(
            ser("［＃ここから2字下げ、ページの左右中央］\nA\n［＃ここで字下げ終わり］"),
            "\n\n［＃ここから2字下げ、ページの左右中央に］\n\nA\n\n［＃ここで字下げ終わり］\n\n"
        );
    }

    #[test]
    fn align_end_block_zero_and_offset() {
        assert_eq!(
            ser("［＃ここから地付き］\nA\n［＃ここで地付き終わり］"),
            "\n\n［＃ここから地付き］\n\nA\n\n［＃ここで地付き終わり］\n\n"
        );
        assert_eq!(
            ser("［＃ここから地から3字上げ］\nA\n［＃ここで地付き終わり］"),
            "\n\n［＃ここから地から3字上げ］\n\nA\n\n［＃ここで地付き終わり］\n\n"
        );
    }

    #[test]
    fn line_width_block_keeps_width() {
        assert_eq!(
            ser("［＃ここから20字詰め］\nA\n［＃ここで字詰め終わり］"),
            "\n\n［＃ここから20字詰め］\n\nA\n\n［＃ここで字詰め終わり］\n\n"
        );
    }

    #[test]
    fn indent_line_kumi_compound_keeps_both_params() {
        // #78: the opener keeps amount + lines + width; the compound closer
        // keeps the width (source-exact, unlike the generic family closers).
        assert_eq!(
            ser("［＃ここから3字下げ、1行20字組みで］\nA\n［＃ここで字下げ、20字組み終わり］"),
            "\n\n［＃ここから3字下げ、1行20字組みで］\n\nA\n\n［＃ここで字下げ、20字組み終わり］\n\n"
        );
    }

    #[test]
    fn indent_line_width_compound_closes_generic() {
        // #78: ここから{N}字下げ、{W}字詰め keeps both params on the opener and
        // closes with the generic 字下げ終わり (matching the corpus).
        assert_eq!(
            ser("［＃ここから8字下げ、18字詰め］\nA\n［＃ここで字下げ終わり］"),
            "\n\n［＃ここから8字下げ、18字詰め］\n\nA\n\n［＃ここで字下げ終わり］\n\n"
        );
    }

    #[test]
    fn keigakomi_block_round_trips() {
        assert_eq!(
            ser("［＃罫囲み］\nA\n［＃罫囲み終わり］"),
            "\n\n［＃罫囲み］\n\nA\n\n［＃罫囲み終わり］\n\n"
        );
    }

    #[test]
    fn bouten_inline_range_folds_to_forward() {
        // S5: a text-only bare 傍点 / 傍線 range folds to the canonical forward
        // leaf (kind + 左に position preserved), and that leaf is a fixed point.
        assert_eq!(ser("［＃傍点］A［＃傍点終わり］"), "A［＃「A」に傍点］");
        assert_eq!(ser("A［＃「A」に傍点］"), "A［＃「A」に傍点］");
        assert_eq!(ser("［＃傍線］A［＃傍線終わり］"), "A［＃「A」に傍線］");
        assert_eq!(
            ser("［＃左に傍線］A［＃左に傍線終わり］"),
            "A［＃「A」の左に傍線］"
        );
        assert_eq!(ser("A［＃「A」の左に傍線］"), "A［＃「A」の左に傍線］");
    }

    /// S5 fold must stay a fixed point on bouten edges the corpus may under-cover.
    /// Bouten serializes through `emit_bouten_targets` (not S4's
    /// `emit_content_as_plain`), so quote-in-target and `、`-bearing targets must
    /// be re-verified, not inferred from S4. The non-foldable cases (点/線 family
    /// mismatch, ruby content) must round-trip verbatim as containers.
    #[test]
    fn s5_fold_round_trips_bouten_edges() {
        for input in [
            "本文［＃傍点］「引用」［＃傍点終わり］。", // quote-in-target → 「「引用」」に傍点
            "［＃二重傍線］乙［＃二重傍線終わり］",     // non-Goma kind
            "［＃左に傍点］X［＃左に傍点終わり］",      // left-side 傍点
            "甲、乙［＃傍点］丙、丁［＃傍点終わり］",   // 、 around + in target (Plain, no split)
            "本文［＃傍点］註［＃傍線終わり］",         // 点/線 mismatch → stays container
            "［＃傍点］｜base《ruby》［＃傍点終わり］", // ruby content → stays container
        ] {
            let once = ser(input);
            let twice = ser(&once);
            assert_eq!(
                once, twice,
                "\n  input: {input}\n  once:  {once}\n  twice: {twice}"
            );
        }
    }

    #[test]
    fn bold_inline_range_folds_to_forward_and_is_idempotent() {
        // S4: a text-only bare ［＃太字］…［＃太字終わり］ range folds to the
        // canonical forward leaf, and that leaf is itself a fixed point.
        assert_eq!(
            ser("本文［＃太字］註［＃太字終わり］。"),
            "本文註［＃「註」は太字］。"
        );
        assert_eq!(
            ser("本文註［＃「註」は太字］。"),
            "本文註［＃「註」は太字］。"
        );
    }

    /// The S4 fold must stay a `parse ∘ serialize` fixed point on the synthetic
    /// edges the corpus does not exercise: targets carrying 「」 quotes (which the
    /// forward form re-quotes as `［＃「「…」」は…］`), embedded quote pairs, and
    /// the non-foldable cases (ruby / nested / crossed / unclosed / stray) that
    /// must round-trip verbatim. Each must satisfy `ser(ser(x)) == ser(x)`.
    #[test]
    fn s4_fold_round_trips_quote_and_structural_edges() {
        for input in [
            // Quote-bearing fold targets — fold, then the forward form is stable.
            "［＃太字］「引用」［＃太字終わり］",
            "［＃斜体］あ「い」う［＃斜体終わり］",
            "［＃キャプション］「図一」と「図二」［＃キャプション終わり］",
            "［＃太字］text ［＃太字終わり］",
            // Non-foldable: ruby content keeps the range a container.
            "［＃太字］｜base《ruby》［＃太字終わり］",
            // Nested: inner folds, outer then holds a non-text child → stays range.
            "a［＃太字］b［＃斜体］c［＃斜体終わり］d［＃太字終わり］",
            // Crossed / mismatched: no fold, every marker survives verbatim.
            "［＃太字］X［＃斜体］Y［＃太字終わり］Z［＃斜体終わり］",
            // Unclosed open and a stray close.
            "［＃太字］tail",
            "head［＃太字終わり］",
        ] {
            let once = ser(input);
            let twice = ser(&once);
            assert_eq!(
                once, twice,
                "\n  input: {input}\n  once:  {once}\n  twice: {twice}"
            );
        }
    }

    #[test]
    fn empty_inline_range_stays_a_container() {
        // No enclosed run → nothing to fold; the bare range round-trips.
        assert_eq!(
            ser("［＃太字］［＃太字終わり］"),
            "［＃太字］［＃太字終わり］"
        );
    }

    #[test]
    fn mismatched_inline_range_close_does_not_fold() {
        // ［＃太字］ closed by ［＃斜体終わり］ is a family mismatch → no fold;
        // both markers survive for the normalizer's mismatch diagnostic.
        assert_eq!(
            ser("本文［＃太字］註［＃斜体終わり］。"),
            "本文［＃太字］註［＃斜体終わり］。"
        );
    }

    #[test]
    fn bold_block_round_trips() {
        assert_eq!(
            ser("［＃ここから太字］\nA\n［＃ここで太字終わり］"),
            "\n\n［＃ここから太字］\n\nA\n\n［＃ここで太字終わり］\n\n"
        );
    }

    #[test]
    fn italic_inline_range_folds_to_forward_and_is_idempotent() {
        assert_eq!(
            ser("本文［＃斜体］註［＃斜体終わり］。"),
            "本文註［＃「註」は斜体］。"
        );
        assert_eq!(
            ser("本文註［＃「註」は斜体］。"),
            "本文註［＃「註」は斜体］。"
        );
    }

    #[test]
    fn italic_block_round_trips() {
        assert_eq!(
            ser("［＃ここから斜体］\nA\n［＃ここで斜体終わり］"),
            "\n\n［＃ここから斜体］\n\nA\n\n［＃ここで斜体終わり］\n\n"
        );
    }

    #[test]
    fn heading_block_round_trips() {
        assert_eq!(
            ser("［＃ここから大見出し］\nA\n［＃ここで大見出し終わり］"),
            "\n\n［＃ここから大見出し］\n\nA\n\n［＃ここで大見出し終わり］\n\n"
        );
    }

    #[test]
    fn heading_paired_window_round_trips() {
        assert_eq!(
            ser("［＃窓中見出し］A［＃窓中見出し終わり］"),
            "\n\n［＃窓中見出し］\n\nA\n\n［＃窓中見出し終わり］\n\n"
        );
    }

    #[test]
    fn columns_block_keeps_count() {
        assert_eq!(
            ser("［＃ここから2段組み］\nA\n［＃ここで段組み終わり］"),
            "\n\n［＃ここから2段組み］\n\nA\n\n［＃ここで段組み終わり］\n\n"
        );
    }

    #[test]
    fn table_block_round_trips() {
        assert_eq!(
            ser("［＃ここから表］\nA\n［＃ここで表終わり］"),
            "\n\n［＃ここから表］\n\nA\n\n［＃ここで表終わり］\n\n"
        );
    }

    #[test]
    fn horizontal_block_round_trips() {
        assert_eq!(
            ser("［＃ここから横組み］\nA\n［＃ここで横組み終わり］"),
            "\n\n［＃ここから横組み］\n\nA\n\n［＃ここで横組み終わり］\n\n"
        );
    }

    #[test]
    fn font_size_block_positive_and_negative() {
        assert_eq!(
            ser("［＃ここから3段階大きな文字］\nA\n［＃ここで大きな文字終わり］"),
            "\n\n［＃ここから3段階大きな文字］\n\nA\n\n［＃ここで大きな文字終わり］\n\n"
        );
        assert_eq!(
            ser("［＃ここから2段階小さな文字］\nA\n［＃ここで小さな文字終わり］"),
            "\n\n［＃ここから2段階小さな文字］\n\nA\n\n［＃ここで小さな文字終わり］\n\n"
        );
    }

    #[test]
    fn small_script_inline_range_folds_to_forward() {
        // S6: the inline 行右 / 行左小書き range folds to its forward leaf, and
        // the forward leaf is a fixed point. Both sides exercise the position axis.
        assert_eq!(
            ser("［＃行右小書き］A［＃行右小書き終わり］"),
            "A［＃「A」は行右小書き］"
        );
        assert_eq!(ser("A［＃「A」は行右小書き］"), "A［＃「A」は行右小書き］");
        assert_eq!(
            ser("［＃行左小書き］A［＃行左小書き終わり］"),
            "A［＃「A」は行左小書き］"
        );
    }

    #[test]
    fn tcy_inline_range_folds_to_forward() {
        // S6: the bare inline 縦中横 range folds; the block (`ここから…`) form and
        // any range spanning a line break (non-text content) stay containers.
        assert_eq!(
            ser("12［＃縦中横］34［＃縦中横終わり］"),
            "1234［＃「34」は縦中横］"
        );
        assert_eq!(ser("1234［＃「34」は縦中横］"), "1234［＃「34」は縦中横］");
    }

    /// S6 fold must stay a fixed point on script / tcy edges: quote-in-target
    /// (via `emit_format`'s `「…」は…` path) and the non-foldable cases (ruby
    /// content stays a container).
    #[test]
    fn s6_fold_round_trips_script_and_tcy_edges() {
        for input in [
            "本文［＃行右小書き］「引」［＃行右小書き終わり］。",
            "本文［＃縦中横］「ロ」［＃縦中横終わり］。",
            "本文［＃縦中横］｜base《ruby》［＃縦中横終わり］。",
        ] {
            let once = ser(input);
            let twice = ser(&once);
            assert_eq!(
                once, twice,
                "\n  input: {input}\n  once:  {once}\n  twice: {twice}"
            );
        }
    }

    #[test]
    fn caption_inline_range_folds_block_stays_container() {
        // S4: the inline bare キャプション range folds to a forward leaf;
        // the block (`ここから…`) form stays a padded container.
        assert_eq!(
            ser("［＃キャプション］A［＃キャプション終わり］"),
            "A［＃「A」はキャプション］"
        );
        assert_eq!(
            ser("A［＃「A」はキャプション］"),
            "A［＃「A」はキャプション］"
        );
        assert_eq!(
            ser("［＃ここからキャプション］\nA\n［＃ここでキャプション終わり］"),
            "\n\n［＃ここからキャプション］\n\nA\n\n［＃ここでキャプション終わり］\n\n"
        );
    }

    #[test]
    fn tcy_range_round_trips() {
        assert_eq!(
            ser("［＃ここから縦中横］\nA\n［＃縦中横終わり］"),
            "［＃ここから縦中横］\nA\n［＃縦中横終わり］"
        );
    }

    // --- A crafted source covering many constructs at once --------------

    #[test]
    fn mixed_document_is_a_fixed_point() {
        let src = concat!(
            "冒頭の文。\n",
            "｜青梅《おうめ》が見える。\n",
            "可哀想［＃「可哀想」に傍点］だ。\n",
            "本文［＃太字］強調［＃太字終わり］。\n",
            "12［＃「12」は縦中横］時。\n",
            "≪引用≫もある。\n",
            "［＃ここから2字下げ］\n字下げ本文\n［＃ここで字下げ終わり］\n",
            "［＃改ページ］\n",
            "末尾。",
        );
        let first = ser(src);
        let second = ser(&first);
        assert_eq!(first, second, "mixed document must reach a fixed point");
        // Spot-check that the headline constructs survived the round-trip
        // (ruby canonicalises to the bare form — `青梅` is an all-kanji base
        // at line start, so the `｜` is dropped per ADR 0002/0003).
        assert!(first.contains("青梅《おうめ》"), "ruby lost: {first:?}");
        assert!(
            first.contains("可哀想［＃「可哀想」に傍点］"),
            "bouten lost: {first:?}"
        );
        assert!(first.contains("≪引用≫"), "angle quote lost: {first:?}");
        assert!(
            first.contains("［＃改ページ］"),
            "page break lost: {first:?}"
        );
    }

    // --- NewlineCappedWriter unit behaviour ----------------------------

    #[test]
    fn newline_capped_writer_caps_runs_at_two() {
        let mut w = NewlineCappedWriter::with_capacity(8);
        w.write_str("a\n\n\n\nb")
            .expect("write into capped writer never fails");
        assert_eq!(w.into_string(), "a\n\nb", "consecutive newlines cap at two");
    }

    #[test]
    fn newline_capped_writer_char_path_caps_runs() {
        let mut w = NewlineCappedWriter::with_capacity(8);
        w.write_char('x').expect("write_char never fails");
        for _ in 0..5 {
            w.write_char('\n').expect("write_char never fails");
        }
        w.write_char('y').expect("write_char never fails");
        assert_eq!(
            w.into_string(),
            "x\n\ny",
            "write_char newline run caps at two"
        );
    }

    #[test]
    fn newline_capped_writer_resets_run_on_text() {
        // A non-newline char between runs resets the counter so each run
        // is capped independently.
        let mut w = NewlineCappedWriter::with_capacity(16);
        w.write_str("\n\n\nA\n\n\n")
            .expect("write into capped writer never fails");
        assert_eq!(w.into_string(), "\n\nA\n\n", "each run caps independently");
    }

    // -------------------------------------------------------------------
    // Coverage for the remaining reachable emit_* arms.
    // -------------------------------------------------------------------

    #[test]
    fn heading_hint_unpromoted_reconstructs_directive() {
        // The referent is not the immediately-preceding bare line, so the
        // forward reference stays a HeadingHint (not a promoted heading);
        // `emit_heading_hint` reconstructs the `［＃「…」は…見出し］` form.
        assert_eq!(
            ser("本文の途中に見出しがある。\n［＃「見出し」は大見出し］"),
            "本文の途中に見出しがある。\n［＃「見出し」は大見出し］"
        );
    }

    #[test]
    fn heading_hint_medium_and_small_levels() {
        assert_eq!(
            ser("長い前置きの文章があって行頭ではない［＃「見出し」は中見出し］"),
            "長い前置きの文章があって行頭ではない［＃「見出し」は中見出し］"
        );
        assert_eq!(
            ser("長い前置きの文章があって行頭ではない［＃「見出し」は小見出し］"),
            "長い前置きの文章があって行頭ではない［＃「見出し」は小見出し］"
        );
    }

    #[test]
    fn bouten_segmented_targets_split_on_comma() {
        // Multiple 「」 targets lex into a `Content::Segments` whose text is
        // comma-joined; `emit_bouten_targets` re-wraps each comma part in
        // its own 「」 → canonical `「甲、乙」` form.
        assert_eq!(
            ser("甲乙［＃「甲」「乙」に傍点］"),
            "甲乙［＃「甲、乙」に傍点］"
        );
    }

    #[test]
    fn gaiji_composed_form_is_emitted_raw() {
        // The composed-glyph description already carries its own 「」, so it
        // must be emitted verbatim (no extra 「」 wrapper).
        assert_eq!(
            ser("※［＃「あ」の「い」に代えて「う」、1-2-3］"),
            "※［＃「あ」の「い」に代えて「う」、1-2-3］"
        );
    }

    #[test]
    fn angle_quote_content_with_gaiji_segment_round_trips() {
        // The AngleQuote body holds a gaiji segment, so `emit_content`
        // walks its `Segment::Gaiji` arm.
        assert_eq!(
            ser("≪※［＃「○」、第3水準1-85-54］≫"),
            "≪※［＃「○」、第3水準1-85-54］≫"
        );
    }
}
