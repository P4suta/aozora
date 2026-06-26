//! Owned-AST Aozora-source serializer.
//!
//! Owned mirror of [`crate::serialize`]: the same single forward walk over the
//! normalized text, but it dispatches each PUA sentinel through an
//! [`OwnedLexOutput`]'s [`RegistryOwned`](aozora_syntax::owned::RegistryOwned)
//! and resolves every [`StrId`](aozora_syntax::owned::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](aozora_syntax::owned::SegRange) against the
//! [`NodeStore`], instead of borrowing `&'a str` / `NodeRef<'a>`.
//!
//! The container-marker spelling (`emit_container_open` / `emit_container_close`)
//! and the writer machinery (`NewlineCappedWriter` / `TrackingWriter`) are
//! **reused** from [`crate::serialize`] — they read only `Copy` `RegionFormat`
//! / `RegionClose` discriminants identical in both worlds, so there is a single
//! byte-spelling authority. Only the AST-reading emitters fork here.
//!
//! It runs the identical decorative-rule isolate post-pass so the byte output
//! matches [`crate::serialize::serialize`] exactly, proven by the differential
//! gate in `crates/aozora/tests/owned_serialize_gate.rs`.

use core::fmt::{self, Write};

use crate::serialize::{
    NewlineCappedWriter, TrackingWriter, emit_container_close, emit_container_open, emit_line,
    emit_section_break, heading_level_word, heading_style_keyword,
};
use crate::walk::{SentinelKind, WalkSinkOwned, walk_owned};
use aozora_pipeline::{has_long_rule_line, isolate_decorative_rules};
use aozora_syntax::borrowed::ForwardOrigin;
use aozora_syntax::owned::{
    AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
    GaijiCanonicalOwned, GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned,
    KaeritenOwned, MarginNoteOwned, NodeOwned, NodeRefOwned, NodeStore, OwnedLexOutput, RubyOwned,
    SegmentOwned,
};
use aozora_syntax::{BoutenPosition, ForwardAttr, RubySide, is_ruby_base_char};

/// Serialize an [`OwnedLexOutput`] back to Aozora source text.
///
/// Owned mirror of [`crate::serialize::serialize`]: a fixed point of
/// `serialize ∘ parse` after one pass. The mandatory decorative-rule isolate
/// post-pass (`has_long_rule_line` fast-path then `isolate_decorative_rules`)
/// is run identically so the byte output matches the borrowed serializer — see
/// the borrowed function's docs for why it holds the round-trip fixed point.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn serialize_owned(out: &OwnedLexOutput) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_owned_into(out, &mut s).expect("writing to NewlineCappedWriter never fails");
    let raw = s.into_string();
    if has_long_rule_line(&raw) {
        isolate_decorative_rules(&raw)
    } else {
        raw
    }
}

/// Serialize an [`OwnedLexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub fn serialize_owned_into<W: Write>(out: &OwnedLexOutput, writer: &mut W) -> fmt::Result {
    let mut tracking = TrackingWriter::new(writer);
    let mut sink = SerializeSinkOwned {
        store: &out.store,
        out: &mut tracking,
    };
    walk_owned(out, &mut sink)
}

/// [`WalkSinkOwned`] that re-emits Aozora source text from the owned AST. Owned
/// mirror of `crate::serialize::SerializeSink`; threads the [`NodeStore`]
/// (the resolve authority) into every AST emitter.
struct SerializeSinkOwned<'a, W: Write> {
    store: &'a NodeStore,
    out: &'a mut TrackingWriter<W>,
}

impl<W: Write> WalkSinkOwned for SerializeSinkOwned<'_, W> {
    // Serialization copies `\n` verbatim, so it is not a structural event.
    const WANTS_NEWLINES: bool = false;

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRefOwned) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRefOwned::Inline(n))
            | (SentinelKind::BlockLeaf, NodeRefOwned::BlockLeaf(n)) => {
                emit_aozora_owned(n, self.store, self.out)
            }
            (SentinelKind::BlockOpen, NodeRefOwned::BlockOpen(open)) => {
                emit_container_open(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRefOwned::BlockClose(close)) => {
                emit_container_close(close, self.out)
            }
            // Sentinel hit without a corresponding registry entry, or a
            // kind/variant mismatch — best-effort skip (mirrors serialize).
            _ => Ok(()),
        }
    }
}

/// Owned mirror of `crate::serialize::emit_aozora` (the 16-arm `Node` match).
fn emit_aozora_owned<W: Write>(
    node: NodeOwned,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
) -> fmt::Result {
    match node {
        NodeOwned::Ruby(r) => emit_ruby_owned(&r, store, out),
        NodeOwned::Format(f) => emit_format_owned(&f, store, out),
        NodeOwned::Gaiji(g) => emit_gaiji_owned(&g, store, out),
        NodeOwned::Kaeriten(k) => emit_kaeriten_owned(k, store, out),
        NodeOwned::Directive(a) => emit_annotation_owned(a, store, out),
        NodeOwned::AngleQuote(d) => emit_angle_quote_owned(d, store, out),
        NodeOwned::MarginNote(s) => emit_side_note_owned(&s, store, out),
        NodeOwned::PageBreak => out.write_str("［＃改ページ］"),
        NodeOwned::BodyEnd => out.write_str("［＃本文終わり］"),
        NodeOwned::ForcedBreak => out.write_str("［＃改行］"),
        NodeOwned::SectionBreak(kind) => emit_section_break(kind, out),
        NodeOwned::Line(lf) => emit_line(lf, out),
        NodeOwned::Illustration(s) => emit_sashie_owned(&s, store, out),
        NodeOwned::HeadingHint(h) => emit_heading_hint_owned(h, store, out),
        NodeOwned::Heading(h) => emit_aozora_heading_owned(&h, store, out),
        // Variants the serializer doesn't cover inline: Container is routed
        // through the open/close sentinel path; Warichu lands here as a
        // diagnostic placeholder, matching the borrowed serializer.
        _ => {
            out.write_str("<!-- unsupported-aozora: ")?;
            out.write_str(node.xml_node_name())?;
            out.write_str(" -->")
        }
    }
}

// ----------------------------------------------------------------------
// Content resolve layer — owned mirror of `emit_content` /
// `emit_content_as_plain` (single `Content` forms + the run forms a
// `NonEmpty<Content>` field maps to).
// ----------------------------------------------------------------------

/// Owned mirror of `crate::serialize::emit_content` for a single
/// [`ContentOwned`].
fn emit_content_one<W: Write>(c: ContentOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        ContentOwned::Plain(id) => out.write_str(store.resolve_str(id)),
        ContentOwned::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    SegmentOwned::Text(id) => out.write_str(store.resolve_str(id))?,
                    SegmentOwned::Gaiji(g) => emit_gaiji_owned(&g, store, out)?,
                    SegmentOwned::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    // `SegmentOwned` is `#[non_exhaustive]`; forward-compat skip.
                    _ => {}
                }
            }
            Ok(())
        }
        // `ContentOwned` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

/// Owned mirror of `emit_content` for a [`ContentRange`]
/// run (a `NonEmpty<Content>` field — length 1 by construction).
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

/// Owned mirror of `crate::serialize::emit_content_as_plain` for a single
/// [`ContentOwned`] (gaiji writes its `hint`, not its glyph form).
fn emit_content_as_plain_one<W: Write>(
    c: ContentOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match c {
        ContentOwned::Plain(id) => out.write_str(store.resolve_str(id)),
        ContentOwned::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    SegmentOwned::Text(id) => out.write_str(store.resolve_str(id))?,
                    SegmentOwned::Gaiji(g) => out.write_str(store.resolve_str(g.hint))?,
                    SegmentOwned::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    // `SegmentOwned` is `#[non_exhaustive]`; forward-compat skip.
                    _ => {}
                }
            }
            Ok(())
        }
        // `ContentOwned` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

/// Owned mirror of `emit_content_as_plain` over a
/// [`ContentRange`] run.
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

// ----------------------------------------------------------------------
// Per-variant AST emitters — owned mirrors of the `emit_*` family.
// ----------------------------------------------------------------------

/// Owned mirror of `crate::serialize::emit_ruby`.
fn emit_ruby_owned<W: Write>(
    r: &RubyOwned,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        emit_content_range(r.base, store, out)?;
        out.write_str("［＃「")?;
        emit_content_range(r.base, store, out)?;
        out.write_str("」の左に「")?;
        emit_content_range(r.reading, store, out)?;
        return out.write_str("」のルビ］");
    }
    if ruby_needs_bar_owned(store.resolve_content_range(r.base), out.last(), store) {
        out.write_char('｜')?;
    }
    emit_content_range(r.base, store, out)?;
    out.write_char('《')?;
    emit_content_range(r.reading, store, out)?;
    out.write_char('》')
}

/// Owned mirror of `crate::serialize::ruby_needs_bar`. A right-side base is
/// always a single `Plain`; the resolved run is matched accordingly.
fn ruby_needs_bar_owned(base_run: &[ContentOwned], prev: Option<char>, store: &NodeStore) -> bool {
    let plain = match base_run {
        [ContentOwned::Plain(id)] => Some(store.resolve_str(*id)),
        _ => None,
    };
    plain.is_none_or(|s| {
        s.chars().any(|c| !is_ruby_base_char(c))
            || prev.is_some_and(|c| is_ruby_base_char(c) || c == '｜')
    })
}

/// Owned mirror of `crate::serialize::emit_format`.
fn emit_format_owned<W: Write>(
    f: &ForwardFormatOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Reclaimed) {
        emit_content_as_plain_range(f.target, store, out)?;
    }
    if let ForwardAttr::Bouten { kind, position } = f.attr {
        out.write_str("［＃")?;
        emit_bouten_targets_owned(store.resolve_content_range(f.target), store, out)?;
        match position {
            BoutenPosition::Left => out.write_str("の左に")?,
            _ => out.write_char('に')?,
        }
        out.write_str(kind.keyword())?;
        return out.write_char('］');
    }
    out.write_str("［＃「")?;
    emit_content_as_plain_range(f.target, store, out)?;
    out.write_str("」は")?;
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

/// Owned mirror of `crate::serialize::emit_bouten_targets`. Operates on the
/// resolved target run (always length 1).
fn emit_bouten_targets_owned<W: Write>(
    run: &[ContentOwned],
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    if let [ContentOwned::Plain(id)] = run {
        out.write_char('「')?;
        out.write_str(store.resolve_str(*id))?;
        return out.write_char('」');
    }
    let mut any = false;
    for c in run {
        if let ContentOwned::Segments(seg_range) = c {
            for seg in store.resolve_seg_range(*seg_range) {
                if let SegmentOwned::Text(id) = *seg {
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

/// Owned mirror of `crate::serialize::emit_gaiji`.
fn emit_gaiji_owned<W: Write>(g: &GaijiOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
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
        out.write_char('、')?;
        write_gaiji_mencode(g.canonical, store, out)?;
    }
    out.write_char('］')
}

/// Owned mirror of `GaijiCanonical::has_mencode`.
const fn gaiji_has_mencode(c: GaijiCanonicalOwned) -> bool {
    !matches!(c, GaijiCanonicalOwned::Unresolved { mencode: None })
}

/// Owned mirror of `GaijiCanonical::write_mencode` — resolves the
/// `Unresolved` tail's [`StrId`](aozora_syntax::owned::StrId) against `store`.
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

/// Owned mirror of `crate::serialize::emit_kaeriten`.
fn emit_kaeriten_owned<W: Write>(k: KaeritenOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(store.resolve_str(k.mark))?;
    out.write_char('］')
}

/// Owned mirror of `crate::serialize::emit_annotation` (raw passthrough; the
/// `raw` bytes already include the `［＃…］` brackets).
fn emit_annotation_owned<W: Write>(
    a: DirectiveOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(store.resolve_str(a.raw))
}

/// Owned mirror of `crate::serialize::emit_angle_quote`.
fn emit_angle_quote_owned<W: Write>(
    d: AngleQuoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_char('≪')?;
    emit_content_range(d.content, store, out)?;
    out.write_char('≫')
}

/// Owned mirror of `crate::serialize::emit_side_note`.
fn emit_side_note_owned<W: Write>(
    s: &MarginNoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    let (connector, suffix) = s.kind.serialize_affixes();
    emit_content_range(s.base, store, out)?;
    out.write_str("［＃「")?;
    emit_content_range(s.base, store, out)?;
    out.write_str(connector)?;
    emit_content_range(s.note, store, out)?;
    out.write_str(suffix)
}

/// Owned mirror of `crate::serialize::emit_sashie`.
fn emit_sashie_owned<W: Write>(
    s: &IllustrationOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
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
        emit_content_as_plain_one(caption, store, out)?;
        out.write_char('」')?;
    }
    out.write_str("入る］")
}

/// Owned mirror of `crate::serialize::emit_heading_hint`.
fn emit_heading_hint_owned<W: Write>(
    h: HeadingHintOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(store.resolve_str(h.target))?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.level))?;
    out.write_str("］")
}

/// Owned mirror of `crate::serialize::emit_aozora_heading`.
fn emit_aozora_heading_owned<W: Write>(
    h: &HeadingOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
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
    use crate::serialize::serialize;
    use crate::serialize_owned::serialize_owned;
    use aozora_syntax::borrowed::Arena;

    /// The owned serializer reproduces the borrowed authority byte-for-byte.
    fn assert_parity(src: &str) {
        let arena = Arena::new();
        let out = aozora_pipeline::lex(src, &arena);
        let owned = out.to_owned();
        assert_eq!(
            serialize_owned(&owned),
            serialize(&out),
            "owned vs borrowed serialize diverged for {src:?}"
        );
    }

    #[test]
    fn owned_matches_borrowed_across_node_kinds() {
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
        ] {
            assert_parity(src);
        }
    }
}
