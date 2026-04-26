//! Borrowed-AST Aozora-source serializer.
//!
//! Mirror of `aozora_parser::serialize`, ported to consume
//! [`aozora_lex::BorrowedLexOutput`] directly. The walk algorithm is
//! identical — single forward `match_indices` over the normalized
//! text, dispatch each PUA sentinel through the borrowed registry,
//! bulk-copy plain runs between hits — and the emitted bytes match
//! the legacy serializer byte-for-byte.
//!
//! Pinned by the `byte_identical_serialize` proptest in
//! `tests/byte_identical_serialize.rs`.

use core::fmt::{self, Write};

use aozora_lex::BorrowedLexOutput;
use aozora_spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::borrowed::{
    Annotation, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, HeadingHint, Kaeriten, Ruby,
    Sashie, Segment, TateChuYoko,
};
use aozora_syntax::{AlignEnd, BoutenKind, BoutenPosition, ContainerKind, Indent, SectionKind};

/// Serialize a `BorrowedLexOutput` back to Aozora source text.
///
/// The output is a fixed point of `serialize ∘ parse` after one
/// pass: a second cycle returns the same bytes. This is the
/// load-bearing corpus-sweep invariant I3 (ADR-0005), preserved in
/// borrowed form.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`Write`] sink.
#[must_use]
pub fn serialize(out: &BorrowedLexOutput<'_>) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_into(out, &mut s).expect("writing to NewlineCappedWriter never fails");
    s.into_string()
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
pub fn serialize_into<W: Write>(out: &BorrowedLexOutput<'_>, writer: &mut W) -> fmt::Result {
    let normalized = out.normalized;
    let registry = &out.registry;

    let mut cursor = 0usize;
    for (pos, sentinel_str) in normalized.match_indices(|c: char| sentinel_kind(c).is_some()) {
        writer.write_str(&normalized[cursor..pos])?;

        let ch = sentinel_str
            .chars()
            .next()
            .expect("match_indices yields non-empty match");
        let byte_pos = u32::try_from(pos).expect("normalized fits u32 per Phase 0 cap");

        match sentinel_kind(ch).expect("predicate matched on this char") {
            SentinelKind::Inline => {
                if let Some(&node) = registry.inline.get(&byte_pos) {
                    emit_aozora(node, writer)?;
                }
            }
            SentinelKind::BlockLeaf => {
                if let Some(&node) = registry.block_leaf.get(&byte_pos) {
                    emit_aozora(node, writer)?;
                }
            }
            SentinelKind::BlockOpen => {
                if let Some(&kind) = registry.block_open.get(&byte_pos) {
                    writer.write_str(container_open_marker(kind))?;
                }
            }
            SentinelKind::BlockClose => {
                if let Some(&kind) = registry.block_close.get(&byte_pos) {
                    writer.write_str(container_close_marker(kind))?;
                }
            }
        }
        cursor = pos + sentinel_str.len();
    }
    writer.write_str(&normalized[cursor..])
}

#[derive(Clone, Copy)]
enum SentinelKind {
    Inline,
    BlockLeaf,
    BlockOpen,
    BlockClose,
}

#[inline]
fn sentinel_kind(c: char) -> Option<SentinelKind> {
    match c {
        INLINE_SENTINEL => Some(SentinelKind::Inline),
        BLOCK_LEAF_SENTINEL => Some(SentinelKind::BlockLeaf),
        BLOCK_OPEN_SENTINEL => Some(SentinelKind::BlockOpen),
        BLOCK_CLOSE_SENTINEL => Some(SentinelKind::BlockClose),
        _ => None,
    }
}

fn emit_aozora<W: Write>(node: AozoraNode<'_>, out: &mut W) -> fmt::Result {
    match node {
        AozoraNode::Ruby(r) => emit_ruby(r, out),
        AozoraNode::Bouten(b) => emit_bouten(b, out),
        AozoraNode::TateChuYoko(t) => emit_tate_chu_yoko(t, out),
        AozoraNode::Gaiji(g) => emit_gaiji(g, out),
        AozoraNode::Kaeriten(k) => emit_kaeriten(k, out),
        AozoraNode::Annotation(a) => emit_annotation(a, out),
        AozoraNode::DoubleRuby(d) => emit_double_ruby(d, out),
        AozoraNode::PageBreak => out.write_str("［＃改ページ］"),
        AozoraNode::SectionBreak(kind) => emit_section_break(kind, out),
        AozoraNode::Indent(i) => emit_indent(i, out),
        AozoraNode::AlignEnd(a) => emit_align_end(a, out),
        AozoraNode::Sashie(s) => emit_sashie(s, out),
        AozoraNode::HeadingHint(h) => emit_heading_hint(h, out),
        // Variants the serializer doesn't yet cover: Container is
        // routed through the open/close sentinel path; Warichu /
        // Keigakomi / AozoraHeading land here as a diagnostic
        // placeholder, matching the legacy serializer's behavior.
        _ => {
            out.write_str("<!-- unsupported-aozora: ")?;
            out.write_str(node.xml_node_name())?;
            out.write_str(" -->")
        }
    }
}

fn emit_ruby<W: Write>(r: &Ruby<'_>, out: &mut W) -> fmt::Result {
    out.write_char('｜')?;
    emit_content(r.base, out)?;
    out.write_char('《')?;
    emit_content(r.reading, out)?;
    out.write_char('》')
}

fn emit_bouten<W: Write>(b: &Bouten<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    emit_bouten_targets(b.target, out)?;
    match b.position {
        BoutenPosition::Left => out.write_str("の左に")?,
        _ => out.write_char('に')?,
    }
    out.write_str(bouten_kind_keyword(b.kind))?;
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

fn emit_tate_chu_yoko<W: Write>(t: &TateChuYoko<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    emit_content_as_plain(t.text, out)?;
    out.write_str("」は縦中横］")
}

fn emit_gaiji<W: Write>(g: &Gaiji<'_>, out: &mut W) -> fmt::Result {
    out.write_char('※')?;
    out.write_str("［＃「")?;
    out.write_str(g.description)?;
    out.write_char('」')?;
    if let Some(m) = g.mencode {
        out.write_char('、')?;
        out.write_str(m)?;
    }
    out.write_char('］')
}

fn emit_kaeriten<W: Write>(k: &Kaeriten<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(k.mark)?;
    out.write_char('］')
}

fn emit_annotation<W: Write>(a: &Annotation<'_>, out: &mut W) -> fmt::Result {
    out.write_str(a.raw)
}

fn emit_double_ruby<W: Write>(d: &DoubleRuby<'_>, out: &mut W) -> fmt::Result {
    out.write_char('《')?;
    out.write_char('《')?;
    emit_content(d.content, out)?;
    out.write_char('》')?;
    out.write_char('》')
}

fn emit_section_break<W: Write>(kind: SectionKind, out: &mut W) -> fmt::Result {
    let keyword = match kind {
        SectionKind::Choho => "改丁",
        SectionKind::Dan => "改段",
        SectionKind::Spread => "改見開き",
        _ => "改ページ",
    };
    out.write_str("［＃")?;
    out.write_str(keyword)?;
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

fn emit_sashie<W: Write>(s: &Sashie<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃挿絵（")?;
    out.write_str(s.file)?;
    out.write_str("）入る］")
}

fn emit_heading_hint<W: Write>(h: &HeadingHint<'_>, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(h.target)?;
    out.write_str(match h.level {
        1 => "」は大見出し］",
        2 => "」は中見出し］",
        _ => "」は小見出し］",
    })
}

const fn container_open_marker(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここから地付き］",
        ContainerKind::Keigakomi => "［＃罫囲み］",
        ContainerKind::Warichu => "［＃割り注］",
        _ => "［＃ここから字下げ］",
    }
}

const fn container_close_marker(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここで地付き終わり］",
        ContainerKind::Keigakomi => "［＃罫囲み終わり］",
        ContainerKind::Warichu => "［＃割り注終わり］",
        _ => "［＃ここで字下げ終わり］",
    }
}

const fn bouten_kind_keyword(kind: BoutenKind) -> &'static str {
    match kind {
        BoutenKind::WhiteSesame => "白ゴマ傍点",
        BoutenKind::Circle => "丸傍点",
        BoutenKind::WhiteCircle => "白丸傍点",
        BoutenKind::DoubleCircle => "二重丸傍点",
        BoutenKind::Janome => "蛇の目傍点",
        BoutenKind::Cross => "ばつ傍点",
        BoutenKind::WhiteTriangle => "白三角傍点",
        BoutenKind::WavyLine => "波線",
        BoutenKind::UnderLine => "傍線",
        BoutenKind::DoubleUnderLine => "二重傍線",
        _ => "傍点",
    }
}

fn emit_content<W: Write>(c: Content<'_>, out: &mut W) -> fmt::Result {
    for seg in c {
        match seg {
            Segment::Text(t) => out.write_str(t)?,
            Segment::Gaiji(g) => emit_gaiji(g, out)?,
            Segment::Annotation(a) => emit_annotation(a, out)?,
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
            Segment::Annotation(a) => out.write_str(a.raw)?,
            _ => {}
        }
    }
    Ok(())
}

/// Output buffer that caps consecutive `\n` runs at two on-the-fly.
///
/// Phase 4 of the lexer pads every block sentinel with `\n\n`
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
        let out = aozora_lex::lex_into_arena(src, &arena);
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
        ];
        for src in inputs {
            let first = ser(src);
            let second = ser(&first);
            assert_eq!(first, second, "fixed point broken for {src:?}");
        }
    }
}
