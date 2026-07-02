//! Owned-AST Aozora-source serializer.
//!
//! Serializes the normalized text back to Aozora source in a single forward
//! walk, dispatching each PUA sentinel through an [`OwnedLexOutput`]'s
//! [`RegistryOwned`](aozora_syntax::owned::RegistryOwned) and resolving every
//! [`StrId`](aozora_syntax::owned::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](aozora_syntax::owned::SegRange) against the [`NodeStore`].
//!
//! The container-marker spelling (`emit_container_open` / `emit_container_close`)
//! and the writer machinery (`NewlineCappedWriter` / `TrackingWriter`) are
//! **reused** from [`crate::serialize`] — they read only `Copy` `RegionFormat`
//! / `RegionClose` discriminants, so there is a single byte-spelling authority.
//! Only the AST-reading emitters live here.
//!
//! It runs a decorative-rule isolate post-pass so `serialize_owned ∘ parse` is
//! a round-trip fixed point.

use core::fmt::{self, Write};

use crate::serialize::{
    NewlineCappedWriter, TrackingWriter, emit_container_close, emit_container_open, emit_line,
    emit_section_break, heading_level_word, heading_style_keyword,
};
use crate::walk::{SentinelKind, WalkSinkOwned, walk_owned};
use aozora_pipeline::{has_long_rule_line, isolate_decorative_rules};
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::lint::canonical_directive;
use aozora_syntax::owned::{
    AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
    GaijiCanonicalOwned, GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned,
    KaeritenOwned, MarginNoteOwned, NodeOwned, NodeRefOwned, NodeStore, OwnedLexOutput, RubyOwned,
    SegmentOwned,
};
use aozora_syntax::{
    AccentMark, BoutenPosition, DirectiveKind, EnclosureKind, ForwardAttr, RubySide,
    is_ruby_base_char,
};

/// Options controlling how the owned AST is re-emitted to Aozora source.
///
/// The default (`fix_notation: false`) preserves the strong contract that
/// every directive round-trips its `raw` bytes verbatim — including the
/// `DirectiveKind::Unknown` near-misses the notation-hygiene lint flags.
/// Opting in (`aozora fmt --fix-notation`) lets the serializer rewrite those
/// flagged near-misses to their canonical spelling via the single
/// [`canonical_directive`] authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SerializeOptions {
    /// Rewrite `DirectiveKind::Unknown` directives whose body is a verified
    /// near-miss (per [`canonical_directive`]) to their canonical spelling.
    pub fix_notation: bool,
}

/// Serialize an [`OwnedLexOutput`] back to Aozora source text.
///
/// `serialize_owned ∘ parse` reaches a fixed point after one pass. The
/// mandatory decorative-rule isolate post-pass (`has_long_rule_line` fast-path
/// then `isolate_decorative_rules`) normalizes decorative rule lines so a
/// second pass re-parses and re-serializes to identical bytes.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn serialize_owned(out: &OwnedLexOutput) -> String {
    serialize_owned_with(out, SerializeOptions::default())
}

/// Serialize an [`OwnedLexOutput`] back to Aozora source text with explicit
/// [`SerializeOptions`].
///
/// With the default options this is identical to [`serialize_owned`]. With
/// `fix_notation` enabled, `DirectiveKind::Unknown` near-misses are rewritten
/// to canonical form (see [`SerializeOptions`]); the rewrite is idempotent
/// because a canonical body parses to a recognized (non-`Unknown`) node and so
/// is never revisited on a second pass.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn serialize_owned_with(out: &OwnedLexOutput, opts: SerializeOptions) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_owned_into_with(out, &mut s, opts)
        .expect("writing to NewlineCappedWriter never fails");
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
    serialize_owned_into_with(out, writer, SerializeOptions::default())
}

/// Serialize an [`OwnedLexOutput`] into the given writer with explicit
/// [`SerializeOptions`].
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub fn serialize_owned_into_with<W: Write>(
    out: &OwnedLexOutput,
    writer: &mut W,
    opts: SerializeOptions,
) -> fmt::Result {
    let mut tracking = TrackingWriter::new(writer);
    let mut sink = SerializeSinkOwned {
        store: &out.store,
        out: &mut tracking,
        fix_notation: opts.fix_notation,
    };
    walk_owned(out, &mut sink)
}

/// [`WalkSinkOwned`] that re-emits Aozora source text from the owned AST,
/// threading the [`NodeStore`] (the resolve authority) into every AST emitter.
struct SerializeSinkOwned<'a, W: Write> {
    store: &'a NodeStore,
    out: &'a mut TrackingWriter<W>,
    /// When set, rewrite `DirectiveKind::Unknown` near-misses to canonical form
    /// (`aozora fmt --fix-notation`).
    fix_notation: bool,
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
                emit_aozora_owned(n, self.store, self.out, self.fix_notation)
            }
            (SentinelKind::BlockOpen, NodeRefOwned::BlockOpen(open)) => {
                emit_container_open(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRefOwned::BlockClose(close)) => {
                emit_container_close(close, self.out)
            }
            // Sentinel hit without a corresponding registry entry, or a
            // kind/variant mismatch — best-effort skip.
            _ => Ok(()),
        }
    }
}

/// Dispatch an owned [`NodeOwned`] to its per-variant source emitter.
fn emit_aozora_owned<W: Write>(
    node: NodeOwned,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
    fix_notation: bool,
) -> fmt::Result {
    match node {
        NodeOwned::Ruby(r) => emit_ruby_owned(&r, store, out),
        NodeOwned::Format(f) => emit_format_owned(&f, store, out),
        NodeOwned::Gaiji(g) => emit_gaiji_owned(&g, store, out),
        NodeOwned::Kaeriten(k) => emit_kaeriten_owned(k, store, out),
        NodeOwned::Directive(a) => emit_annotation_owned(a, store, out, fix_notation),
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
        // Variants not covered inline: Container is routed through the
        // open/close sentinel path; Warichu lands here as a diagnostic
        // placeholder.
        _ => {
            out.write_str("<!-- unsupported-aozora: ")?;
            out.write_str(node.xml_node_name())?;
            out.write_str(" -->")
        }
    }
}

// ----------------------------------------------------------------------
// Content resolve layer — resolve and emit a `ContentOwned` / `ContentRange`,
// either verbatim or as plain text (gaiji written as its hint).
// ----------------------------------------------------------------------

/// Emit a single [`ContentOwned`] verbatim (plain text, or each segment's
/// text / gaiji / directive).
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

/// Emit a [`ContentRange`] run (length 1 by construction) by serializing each
/// resolved [`ContentOwned`].
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

/// Emit a single [`ContentOwned`] as plain text: a gaiji segment writes its
/// `hint`, not its glyph form.
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

/// Emit a [`ContentRange`] run as plain text.
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
// Per-variant AST emitters.
// ----------------------------------------------------------------------

/// Serialize a ruby node: a left-side ruby to its
/// `base［＃「base」の左に「reading」のルビ］` form; a right-side ruby to
/// `base《reading》`, prefixed with `｜` when the base needs an explicit bar.
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

/// Decide whether a right-side ruby base needs an explicit `｜` start bar: true
/// when the base contains a non-ruby-base character, or the preceding char is
/// itself a ruby-base character or `｜`. A right-side base is always a single
/// `Plain`; the resolved run is matched accordingly.
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

/// Serialize a forward-format node to its `［＃…］` bracket form. A `Reclaimed`
/// origin first re-emits the target literal; a bouten attribute uses the
/// `…に<keyword>` (or `の左に`) shape; a font-size attribute spells its
/// `N段階大きな/小さな文字` magnitude; every other attribute uses
/// `［＃「target」は<keyword>］`.
fn emit_format_owned<W: Write>(
    f: &ForwardFormatOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Reclaimed | ForwardOrigin::Detached) {
        emit_content_as_plain_range(f.target, store, out)?;
    }
    // A `Detached` decoration (#333) is the styled-literal half of a
    // non-adjacent forward split: it serializes as the bare literal above,
    // because the `［＃…］` directive is a *separate* `Referenced` node that
    // emits the bracket itself. Return before the bracket-emitting block.
    if matches!(f.origin, ForwardOrigin::Detached) {
        return Ok(());
    }
    if let ForwardAttr::Bouten { kind, position } = f.attr {
        out.write_str("［＃")?;
        emit_bouten_targets_owned(store.resolve_content_range(f.target), store, out)?;
        match position {
            BoutenPosition::Left => out.write_str("の左に")?,
            BoutenPosition::Both => out.write_str("の両側に")?,
            _ => out.write_char('に')?,
        }
        out.write_str(kind.keyword())?;
        return out.write_char('］');
    }
    if matches!(f.attr, ForwardAttr::Framed(EnclosureKind::Box)) {
        // 「□」囲み: the keyword embeds the quoted glyph, so it can't come from
        // `keyword()`. □ (U+25A1) is the canonical spelling of the Box kind.
        out.write_str("［＃「")?;
        emit_content_as_plain_range(f.target, store, out)?;
        return out.write_str("」は「□」囲み］");
    }
    if matches!(f.attr, ForwardAttr::AccentDot) {
        // ドット付き (#331): the body is a selector grammar, not the
        // `「target」は<keyword>` shape, so re-emit the interned raw body verbatim
        // (byte-exact round-trip). The `Reclaimed` leading literal — the run the
        // dots compose onto — was already emitted above.
        out.write_str("［＃")?;
        if let Some(id) = f.accent_body {
            out.write_str(store.resolve_str(id))?;
        }
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
    } else if let ForwardAttr::AlignEnd { offset } = f.attr {
        // Anchor is not distinguished in the model (like LineFormat::AlignEnd);
        // canonicalise to 文末より…字上げ揃え, which re-parses to the same offset.
        write!(out, "文末より{offset}字上げ揃え")?;
    } else if let ForwardAttr::Accent(mark) = f.attr {
        // アクサン / ウムラウト: the suffix carries the bracketed mark symbol, not a
        // bare keyword (so `keyword()` returns its 太字 default) — re-emit the
        // exact source suffix for a byte-exact round-trip.
        out.write_str(accent_suffix(mark))?;
    } else {
        out.write_str(f.attr.keyword())?;
    }
    out.write_char('］')
}

/// The exact `は`-suffix source for a forward accent [`AccentMark`], for a
/// byte-exact round-trip: fullwidth parens (U+FF08 / U+FF09) wrapping the mark
/// symbol (´ U+00B4 / ｀ U+FF40 / ¨ U+00A8).
const fn accent_suffix(mark: AccentMark) -> &'static str {
    match mark {
        AccentMark::Acute => "アクサン（´）付き",
        AccentMark::Grave => "アクサン（｀）付き",
        AccentMark::Umlaut => "ウムラウト（¨）付き",
    }
}

/// Serialize the bouten target(s) as quoted `「…」` runs. A single `Plain`
/// target becomes one `「text」`; a segmented target is split on `、` into one
/// `「part」` per piece (falling back to an empty `「」`). Operates on the
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

/// Serialize a gaiji node to its `［＃「hint」…］` bracket form (prefixed with
/// `※` unless `standalone`), appending the mencode tail when the canonical
/// form carries one.
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

/// Serialize a kaeriten mark to its `［＃<mark>］` bracket form.
fn emit_kaeriten_owned<W: Write>(k: KaeritenOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(store.resolve_str(k.mark))?;
    out.write_char('］')
}

/// Serialize a directive by writing its `raw` bytes verbatim (they already
/// include the `［＃…］` brackets).
///
/// With `fix_notation` set, an `Unknown` directive whose trimmed body is a
/// verified near-miss (per [`canonical_directive`]) is rewritten to
/// `［＃<canonical>］` instead. The rewrite is idempotent: the canonical body
/// parses to a recognized (non-`Unknown`) node, so a second serialize pass
/// never re-enters this branch and re-emits verbatim.
fn emit_annotation_owned<W: Write>(
    a: DirectiveOwned,
    store: &NodeStore,
    out: &mut W,
    fix_notation: bool,
) -> fmt::Result {
    let raw = store.resolve_str(a.raw);
    if fix_notation && a.kind == DirectiveKind::Unknown {
        let body = raw
            .strip_prefix("［＃")
            .and_then(|s| s.strip_suffix('］'))
            .unwrap_or(raw)
            .trim();
        if let Some(canonical) = canonical_directive(body) {
            out.write_str("［＃")?;
            out.write_str(canonical.as_ref())?;
            return out.write_char('］');
        }
    }
    out.write_str(raw)
}

/// Serialize an angle-quote to its `≪…≫` form.
fn emit_angle_quote_owned<W: Write>(
    d: AngleQuoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_char('≪')?;
    emit_content_range(d.content, store, out)?;
    out.write_char('≫')
}

/// Serialize a margin note to its `base［＃「base」…］` form, using the kind's
/// connector / suffix affixes around the note text.
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

/// Serialize an illustration to its `［＃…（file）…入る］` bracket form (a
/// description, or `挿絵` + optional number; optional dimensions and caption).
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

/// Serialize a heading hint back to its `［＃「X」は…見出し］` bracket form.
///
/// `self_contained` is deliberately ignored: a no-referent forward heading
/// serializes to the bracket alone, never fabricating a referent line. That
/// keeps the round-trip a fixed point (a fabricated line would re-parse as a
/// promotable referent — see `promote_headings` — diverging on the next pass).
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

/// Serialize an Aozora heading to its referent line followed by the
/// `［＃「text」は<style><level>見出し］` bracket.
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
    use crate::serialize_owned::serialize_owned;

    /// `serialize_owned ∘ parse` reaches a fixed point after one pass — the
    /// canonical round-trip contract (end-to-end byte-identity is pinned by the
    /// conformance golden).
    fn assert_parity(src: &str) {
        let first = serialize_owned(&aozora_pipeline::lex(src));
        let second = serialize_owned(&aozora_pipeline::lex(&first));
        assert_eq!(
            first, second,
            "serialize_owned fixed point diverged for {src:?}"
        );
    }

    #[test]
    fn owned_serialize_is_fixed_point_across_node_kinds() {
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

    /// E1-1: a no-referent forward ([`ForwardOrigin::SelfContained`]) is not
    /// [`ForwardOrigin::Reclaimed`], so the serializer emits **no** leading
    /// literal — just the `［＃「X」は太字］` bracket. (`Reclaimed` would prefix
    /// the literal `X`.) Pinned directly since the producer arrives in E1-2.
    #[test]
    fn self_contained_forward_serializes_bracket_only() {
        use aozora_syntax::alloc_owned::OwnedAllocator;
        use aozora_syntax::owned::NodeOwned;
        use aozora_syntax::{ForwardAttr, ForwardOrigin};

        let mut a = OwnedAllocator::new();
        let t = a.content_plain("X");
        let node = a.forward_format(ForwardAttr::Bold, t, ForwardOrigin::SelfContained);
        let NodeOwned::Format(f) = node else {
            panic!("forward_format must build a Format node");
        };
        let store = a.into_store();

        let mut s = String::new();
        super::emit_format_owned(&f, &store, &mut s).expect("serialize into String is infallible");
        assert_eq!(s, "［＃「X」は太字］");
    }

    /// E1-4: a self-contained heading hint serializes to the bracket alone — it
    /// must NOT fabricate a referent line, which would re-parse as a promotable
    /// heading (`promote_headings`) and break the round-trip fixed point.
    #[test]
    fn self_contained_heading_serializes_bracket_only() {
        use aozora_syntax::alloc_owned::OwnedAllocator;
        use aozora_syntax::owned::NodeOwned;
        use aozora_syntax::{HeadingKind, HeadingStyle};

        let mut a = OwnedAllocator::new();
        let node = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章", true);
        let NodeOwned::HeadingHint(h) = node else {
            panic!("heading_hint must build a HeadingHint node");
        };
        let store = a.into_store();

        let mut s = String::new();
        super::emit_heading_hint_owned(h, &store, &mut s)
            .expect("serialize into String is infallible");
        assert_eq!(s, "［＃「序章」は中見出し］");
    }
}
