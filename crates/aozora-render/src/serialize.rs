//! Borrowed-AST Aozora-source serializer.
//!
//! Single forward `match_indices` over the normalized text, dispatch
//! each PUA sentinel through the borrowed registry, bulk-copy plain
//! runs between hits.
//!
//! Round-trip fixed-point pinned by the `byte_identical_serialize`
//! proptest in `tests/byte_identical_serialize.rs`.

use core::fmt::{self, Write};

use crate::walk::{SentinelKind, WalkSink, walk};
use aozora_pipeline::{LexOutput, has_long_rule_line, isolate_decorative_rules};
use aozora_syntax::borrowed::{
    AngleQuote, Bouten, CombineUpright, Content, Directive, Emphasis, Gaiji, Heading, HeadingHint,
    Illustration, Kaeriten, MarginNote, Node, NodeRef, Ruby, Segment,
};
use aozora_syntax::{
    AlignEnd, BoutenPosition, Center, ContainerKind, EmphasisKind, HeadingKind, HeadingStyle,
    Indent, IndentLayout, RubySide, SectionKind,
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
    let mut sink = SerializeSink { out: writer };
    walk(out, &mut sink)
}

/// [`WalkSink`] that re-emits Aozora source text: plain runs are copied
/// verbatim (newlines included — [`Self::WANTS_NEWLINES`] is `false`) and
/// each sentinel is reconstructed through the `emit_*` helpers.
struct SerializeSink<'w, W: Write> {
    out: &'w mut W,
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
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(kind)) => {
                emit_container_open(kind, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(kind)) => {
                emit_container_close(kind, self.out)
            }
            // Sentinel hit without a corresponding registry entry, or a
            // kind/variant mismatch — best-effort skip (the per-table
            // lookups silently dropped these too).
            _ => Ok(()),
        }
    }
}

fn emit_aozora<W: Write>(node: Node<'_>, out: &mut W) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_ruby(r, out),
        Node::Bouten(b) => emit_bouten(b, out),
        Node::CombineUpright(t) => emit_tate_chu_yoko(t, out),
        Node::Gaiji(g) => emit_gaiji(g, out),
        Node::Kaeriten(k) => emit_kaeriten(k, out),
        Node::Directive(a) => emit_annotation(a, out),
        Node::AngleQuote(d) => emit_angle_quote(d, out),
        Node::Emphasis(e) => emit_emphasis(e, out),
        Node::MarginNote(s) => emit_side_note(s, out),
        Node::PageBreak => out.write_str("［＃改ページ］"),
        Node::SectionBreak(kind) => emit_section_break(kind, out),
        Node::Indent(i) => emit_indent(i, out),
        Node::AlignEnd(a) => emit_align_end(a, out),
        Node::Center(c) => emit_center(c, out),
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

fn emit_ruby<W: Write>(r: &Ruby<'_>, out: &mut W) -> fmt::Result {
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
    out.write_char('｜')?;
    emit_content(r.base.get(), out)?;
    out.write_char('《')?;
    emit_content(r.reading.get(), out)?;
    out.write_char('》')
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

fn emit_bouten<W: Write>(b: &Bouten<'_>, out: &mut W) -> fmt::Result {
    if b.consumed_predecessor {
        // The classify stage pulled this node's source span back over the literal
        // occurrence of `target` that sat immediately before the `［`.
        // Re-emit the literal so the serialized output round-trips back
        // to the original source: `<target>［＃「<target>」に傍点］`
        // becomes the canonical fixed-point shape.
        emit_content_as_plain(b.target.get(), out)?;
    }
    out.write_str("［＃")?;
    emit_bouten_targets(b.target.get(), out)?;
    match b.position {
        BoutenPosition::Left => out.write_str("の左に")?,
        _ => out.write_char('に')?,
    }
    out.write_str(b.kind.keyword())?;
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

fn emit_tate_chu_yoko<W: Write>(t: &CombineUpright<'_>, out: &mut W) -> fmt::Result {
    if t.consumed_predecessor {
        // Same back-ref re-emit as `emit_bouten` — see that function's
        // comment for the round-trip rationale.
        emit_content_as_plain(t.text.get(), out)?;
    }
    out.write_str("［＃「")?;
    emit_content_as_plain(t.text.get(), out)?;
    out.write_str("」は縦中横］")
}

/// Re-emit a forward-reference 太字 / 斜体 leaf as
/// `<text>［＃「<text>」は太字／斜体］`. `consumed_predecessor` drives the
/// leading literal re-emit, identical to `emit_bouten` / `emit_tate_chu_yoko`.
fn emit_emphasis<W: Write>(e: &Emphasis<'_>, out: &mut W) -> fmt::Result {
    if e.consumed_predecessor {
        emit_content_as_plain(e.text.get(), out)?;
    }
    out.write_str("［＃「")?;
    emit_content_as_plain(e.text.get(), out)?;
    out.write_str("」は")?;
    // 文字サイズ carries a magnitude that the static keyword table can't hold.
    if let EmphasisKind::FontSize { steps } = e.kind {
        let (magnitude, word) = if steps >= 0 {
            (steps, "大きな")
        } else {
            (-steps, "小さな")
        };
        write!(out, "{magnitude}段階{word}文字")?;
    } else {
        out.write_str(e.kind.keyword())?;
    }
    out.write_char('］')
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
    if g.description.contains(['「', '」']) {
        out.write_str(g.description)?;
    } else {
        out.write_char('「')?;
        out.write_str(g.description)?;
        out.write_char('」')?;
    }
    if let Some(m) = g.mencode {
        out.write_char('、')?;
        out.write_str(m)?;
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

fn emit_section_break<W: Write>(kind: SectionKind, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(kind.keyword())?;
    out.write_char('］')
}

fn emit_indent<W: Write>(i: Indent, out: &mut W) -> fmt::Result {
    if i.amount == 1 {
        out.write_str("［＃字下げ］")
    } else {
        write!(out, "［＃{}字下げ］", i.amount)
    }
}

fn emit_align_end<W: Write>(a: AlignEnd, out: &mut W) -> fmt::Result {
    if a.offset == 0 {
        out.write_str("［＃地付き］")
    } else {
        write!(out, "［＃地から{}字上げ］", a.offset)
    }
}

fn emit_center<W: Write>(c: Center, out: &mut W) -> fmt::Result {
    out.write_str(if c.page {
        "［＃ページの左右中央］"
    } else {
        "［＃中央揃え］"
    })
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
const fn heading_style_keyword(style: HeadingStyle) -> &'static str {
    match style {
        HeadingStyle::SameLine => "同行",
        HeadingStyle::Window => "窓",
        // Standard and any future style serialize without a prefix.
        _ => "",
    }
}

/// The `大 / 中 / 小見出し` level keyword (no delimiter), shared by the leaf
/// heading, the hint, and the paired / block [`ContainerKind::Heading`].
const fn heading_level_word(kind: HeadingKind) -> &'static str {
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
    out.write_str(match h.level {
        2 => "中見出し",
        3 => "小見出し",
        _ => "大見出し",
    })?;
    out.write_str("］")
}

const fn container_open_marker(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここから地付き］",
        ContainerKind::Framed => "［＃罫囲み］",
        ContainerKind::Warichu => "［＃割り注］",
        ContainerKind::CombineUprightRange => "［＃縦中横］",
        _ => "［＃ここから字下げ］",
    }
}

const fn container_close_marker(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここで地付き終わり］",
        ContainerKind::Framed => "［＃罫囲み終わり］",
        ContainerKind::Warichu => "［＃割り注終わり］",
        ContainerKind::CombineUprightRange => "［＃縦中横終わり］",
        _ => "［＃ここで字下げ終わり］",
    }
}

/// `左に` left-side prefix for a bouten range marker, or `""`.
const fn bouten_left_prefix(position: BoutenPosition) -> &'static str {
    match position {
        BoutenPosition::Left => "左に",
        _ => "",
    }
}

/// Serialize a container open marker. 傍点 / 傍線 ranges reconstruct
/// `［＃<左に?><variant>］`; the fixed-family containers use the static
/// [`container_open_marker`].
fn emit_container_open<W: Write>(kind: ContainerKind, out: &mut W) -> fmt::Result {
    match kind {
        ContainerKind::BoutenRange { kind, position } => write!(
            out,
            "［＃{}{}］",
            bouten_left_prefix(position),
            kind.keyword()
        ),
        // #78 line-layout compounds — checked first so the `..`-tolerant plain
        // arms below cannot swallow a layout-bearing Indent and silently drop
        // the secondary clause (a §7.6 fixed-point violation).
        ContainerKind::Indent {
            amount,
            layout: IndentLayout::Kumi { lines, width },
            ..
        } => write!(
            out,
            "［＃ここから{amount}字下げ、{lines}行{width}字組みで］"
        ),
        ContainerKind::Indent {
            amount,
            layout: IndentLayout::LineWidth(width),
            ..
        } => write!(out, "［＃ここから{amount}字下げ、{width}字詰め］"),
        ContainerKind::Indent {
            amount,
            wrap: Some(wrap),
            layout: IndentLayout::None,
            ..
        } => write!(out, "［＃ここから{amount}字下げ、折り返して{wrap}字下げ］"),
        // Combined 字下げ＋ページ左右中央 — an indented, page-centred block.
        ContainerKind::Indent {
            amount,
            wrap: None,
            center: true,
            layout: IndentLayout::None,
        } => write!(out, "［＃ここから{amount}字下げ、ページの左右中央に］"),
        // Plain 字下げ — preserve the amount. A bare container_open_marker
        // fallback collapses it to ［＃ここから字下げ］, dropping N (a §7.6
        // fixed-point violation). `amount == 1` keeps the idiomatic
        // no-number 字下げ form (the IndentBlock1 opener).
        ContainerKind::Indent {
            amount: 1,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
        } => out.write_str("［＃ここから字下げ］"),
        ContainerKind::Indent {
            amount,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
        } => write!(out, "［＃ここから{amount}字下げ］"),
        ContainerKind::Bold { block: false } => out.write_str("［＃太字］"),
        ContainerKind::Bold { block: true } => out.write_str("［＃ここから太字］"),
        ContainerKind::Italic { block: false } => out.write_str("［＃斜体］"),
        ContainerKind::Italic { block: true } => out.write_str("［＃ここから斜体］"),
        // Preserve the 地から N 字上げ offset. A bare fallback collapses
        // every AlignEnd opener to ［＃ここから地付き］, silently dropping a
        // non-zero offset (a §7.6 fixed-point violation). The close marker
        // canonicalises to ［＃ここで地付き終わり］ for both forms (the
        // close node carries no offset; the open-side payload is
        // authoritative).
        ContainerKind::AlignEnd { offset: 0 } => out.write_str("［＃ここから地付き］"),
        ContainerKind::AlignEnd { offset } => write!(out, "［＃ここから地から{offset}字上げ］"),
        // Preserve the width (byte-exact). A bare fallback would emit the
        // 字下げ opener and silently mislabel the family.
        ContainerKind::LineWidth { width } => write!(out, "［＃ここから{width}字詰め］"),
        ContainerKind::Heading { kind, style, block } => write!(
            out,
            "［＃{}{}{}］",
            if block { "ここから" } else { "" },
            heading_style_keyword(style),
            heading_level_word(kind),
        ),
        ContainerKind::Columns { count } => write!(out, "［＃ここから{count}段組み］"),
        ContainerKind::Table => out.write_str("［＃ここから表］"),
        ContainerKind::Horizontal => out.write_str("［＃ここから横組み］"),
        ContainerKind::FontSize { steps } => {
            let (magnitude, word) = if steps >= 0 {
                (steps, "大きな")
            } else {
                (-steps, "小さな")
            };
            write!(out, "［＃ここから{magnitude}段階{word}文字］")
        }
        ContainerKind::SmallScript { side } => {
            write!(out, "［＃行{}小書き］", small_script_side_word(side))
        }
        ContainerKind::Caption { block } => out.write_str(if block {
            "［＃ここからキャプション］"
        } else {
            "［＃キャプション］"
        }),
        // `ContainerKind::Warichu` is the block 割り注 region (the inline
        // ［＃割り注］ is an `Directive{WarichuOpen}`), so it serializes to
        // the ここから form.
        ContainerKind::Warichu => out.write_str("［＃ここから割り注］"),
        _ => out.write_str(container_open_marker(kind)),
    }
}

/// 小書き side keyword: `右` / `左`.
const fn small_script_side_word(side: BoutenPosition) -> &'static str {
    match side {
        BoutenPosition::Left => "左",
        _ => "右",
    }
}

/// Serialize a container close marker — the bouten range close adds the
/// `終わり` suffix to the same `［＃<左に?><variant>…］` form.
fn emit_container_close<W: Write>(kind: ContainerKind, out: &mut W) -> fmt::Result {
    match kind {
        ContainerKind::BoutenRange { kind, position } => write!(
            out,
            "［＃{}{}終わり］",
            bouten_left_prefix(position),
            kind.keyword()
        ),
        ContainerKind::Bold { block: false } => out.write_str("［＃太字終わり］"),
        ContainerKind::Bold { block: true } => out.write_str("［＃ここで太字終わり］"),
        ContainerKind::Italic { block: false } => out.write_str("［＃斜体終わり］"),
        ContainerKind::Italic { block: true } => out.write_str("［＃ここで斜体終わり］"),
        // #78 字組み compound — the close marker carries the width so it
        // round-trips byte-exact (unlike the other block closers, which the
        // open side keeps authoritative). The 字詰め compound and the plain /
        // 折り返して / 中央 indents all fall to the generic 字下げ終わり below.
        ContainerKind::Indent {
            layout: IndentLayout::Kumi { width, .. },
            ..
        } => write!(out, "［＃ここで字下げ、{width}字組み終わり］"),
        ContainerKind::LineWidth { .. } => out.write_str("［＃ここで字詰め終わり］"),
        ContainerKind::Heading { kind, style, block } => write!(
            out,
            "［＃{}{}{}終わり］",
            if block { "ここで" } else { "" },
            heading_style_keyword(style),
            heading_level_word(kind),
        ),
        ContainerKind::Columns { .. } => out.write_str("［＃ここで段組み終わり］"),
        ContainerKind::Table => out.write_str("［＃ここで表終わり］"),
        ContainerKind::Horizontal => out.write_str("［＃ここで横組み終わり］"),
        ContainerKind::FontSize { steps } => out.write_str(if steps >= 0 {
            "［＃ここで大きな文字終わり］"
        } else {
            "［＃ここで小さな文字終わり］"
        }),
        ContainerKind::SmallScript { side } => {
            write!(out, "［＃行{}小書き終わり］", small_script_side_word(side))
        }
        ContainerKind::Caption { block } => out.write_str(if block {
            "［＃ここでキャプション終わり］"
        } else {
            "［＃キャプション終わり］"
        }),
        ContainerKind::Warichu => out.write_str("［＃ここで割り注終わり］"),
        _ => out.write_str(container_close_marker(kind)),
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
            Segment::Gaiji(g) => out.write_str(g.description)?,
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
struct NewlineCappedWriter {
    out: String,
    trailing_newlines: usize,
}

impl NewlineCappedWriter {
    fn with_capacity(cap: usize) -> Self {
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

    fn into_string(self) -> String {
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
        let out = ser("｜青梅《おうめ》");
        assert!(out.contains("｜青梅《おうめ》"), "got {out:?}");
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
    fn explicit_ruby_exact() {
        assert_eq!(ser("｜青梅《おうめ》"), "｜青梅《おうめ》");
    }

    #[test]
    fn implicit_ruby_canonicalises_to_explicit_delimiter() {
        // No `｜`: the serializer re-emits the canonical explicit form.
        assert_eq!(ser("青梅《おうめ》"), "｜青梅《おうめ》");
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
    fn bouten_range_inline_and_left_round_trip() {
        assert_eq!(
            ser("［＃傍点］A［＃傍点終わり］"),
            "［＃傍点］A［＃傍点終わり］"
        );
        assert_eq!(
            ser("［＃傍線］A［＃傍線終わり］"),
            "［＃傍線］A［＃傍線終わり］"
        );
        assert_eq!(
            ser("［＃左に傍線］A［＃左に傍線終わり］"),
            "［＃左に傍線］A［＃左に傍線終わり］"
        );
    }

    #[test]
    fn bold_inline_range_round_trips() {
        assert_eq!(
            ser("本文［＃太字］註［＃太字終わり］。"),
            "本文［＃太字］註［＃太字終わり］。"
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
    fn italic_inline_range_round_trips() {
        assert_eq!(
            ser("本文［＃斜体］註［＃斜体終わり］。"),
            "本文［＃斜体］註［＃斜体終わり］。"
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
    fn small_script_range_round_trips() {
        assert_eq!(
            ser("［＃行右小書き］A［＃行右小書き終わり］"),
            "［＃行右小書き］A［＃行右小書き終わり］"
        );
        assert_eq!(
            ser("［＃行左小書き］A［＃行左小書き終わり］"),
            "［＃行左小書き］A［＃行左小書き終わり］"
        );
    }

    #[test]
    fn caption_range_and_block_round_trip() {
        assert_eq!(
            ser("［＃キャプション］A［＃キャプション終わり］"),
            "［＃キャプション］A［＃キャプション終わり］"
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
        // Spot-check that the headline constructs survived the round-trip.
        assert!(first.contains("｜青梅《おうめ》"), "ruby lost: {first:?}");
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
