//! Aozora-source serializer over the semantic AST.
//!
//! Serializes the normalized text back to Aozora source in a single forward
//! walk, dispatching each PUA sentinel through an [`LexOutput`]'s
//! [`Registry`](aozora_syntax::ast::Registry) and resolving every
//! [`StrId`](aozora_syntax::ast::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](aozora_syntax::ast::SegRange) against the [`NodeStore`].
//!
//! The container-marker spelling (`emit_container_open` / `emit_container_close`)
//! and the writer machinery (`NewlineCappedWriter` / `TrackingWriter`) are
//! **reused** from [`crate::spelling::source`] — they read only `Copy` `RegionFormat`
//! / `RegionClose` discriminants, so there is a single byte-spelling authority.
//! Only the AST-reading emitters live here.
//!
//! It runs a decorative-rule isolate post-pass so `serialize ∘ parse` is
//! a round-trip fixed point.

use core::fmt::{self, Write};

use crate::spelling::source::{
    NewlineCappedWriter, TrackingWriter, emit_container_close, emit_container_open, emit_line,
    emit_section_break, heading_level_word, heading_style_keyword,
};
use crate::walk::{SentinelKind, WalkSink, walk};
use aozora_pipeline::{has_long_rule_line, isolate_decorative_rules};
use aozora_syntax::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardFormat, Gaiji, GaijiCanonicalOwned,
    Heading, HeadingHint, Illustration, Kaeriten, LexOutput, MarginNote, Node, NodeRef, NodeStore,
    Ruby, Segment,
};
use aozora_syntax::degraded::degraded_directive;
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::lint::canonical_directive;
use aozora_syntax::{
    AccentMark, BoutenPosition, DirectiveKind, EnclosureKind, ForwardAttr, RubySide,
    ruby_base_class,
};

/// Which notation-hygiene catalogue tiers a serialize / render pass consults
/// when it meets a `DirectiveKind::Unknown` near-miss.
///
/// The default (`Off`) is the byte-identical, non-judgemental path: an Unknown
/// directive round-trips its raw bytes verbatim (serialize) / renders as an
/// inert `<span class="aozora-directive" hidden>` (render), so output never
/// depends on the catalogue. The three levels map exactly to the tiers of
/// ADR-0022 / ADR-0026.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectiveNormalization {
    /// Verbatim / inert — the byte-identical default.
    #[default]
    Off,
    /// Tier1 only: rewrite verified zero-false-positive near-misses (per
    /// [`canonical_directive`]) to canonical form. The level `fmt --fix`
    /// and `render --normalize` use.
    Canonical,
    /// Tier1 + Tier2: additionally reduce the lossy / judgment degraded forms
    /// (per [`degraded_directive`]) Tier1 refuses. Constructed **only** by the
    /// opt-in renderer ([`crate::render_html_normalized`] via `render --degraded`),
    /// never by a persistent-write path, so a Tier2 misfire can reach only
    /// `--degraded` render output — never source. See ADR-0026.
    Degraded,
}

/// Options controlling how the AST is re-emitted to Aozora source.
///
/// The default (`directives: Off`) preserves the strong contract that every
/// directive round-trips its `raw` bytes verbatim — including the
/// `DirectiveKind::Unknown` near-misses the notation-hygiene lint flags.
/// Opting in (`aozora fmt --fix` = `Canonical`) lets the serializer
/// rewrite those flagged near-misses to their canonical spelling via the single
/// [`canonical_directive`] authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SerializeOptions {
    /// Which notation-hygiene tiers to consult for `DirectiveKind::Unknown`
    /// near-misses. `Degraded` is reserved for the opt-in renderer.
    pub directives: DirectiveNormalization,
}

/// Serialize an [`LexOutput`] back to Aozora source text.
///
/// `serialize ∘ parse` reaches a fixed point after one pass. The
/// mandatory decorative-rule isolate post-pass (`has_long_rule_line` fast-path
/// then `isolate_decorative_rules`) normalizes decorative rule lines so a
/// second pass re-parses and re-serializes to identical bytes.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn serialize(out: &LexOutput) -> String {
    serialize_with(out, SerializeOptions::default())
}

/// Serialize an [`LexOutput`] back to Aozora source text with explicit
/// [`SerializeOptions`].
///
/// With the default options this is identical to [`serialize()`]. With
/// `directives` not `Off`, `DirectiveKind::Unknown` near-misses are rewritten
/// to canonical form (see [`SerializeOptions`]); the rewrite is idempotent
/// because a canonical body parses to a recognized (non-`Unknown`) node and so
/// is never revisited on a second pass.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`Write`] sink.
#[must_use]
pub fn serialize_with(out: &LexOutput, opts: SerializeOptions) -> String {
    let mut s = NewlineCappedWriter::with_capacity(out.normalized.len().saturating_mul(2));
    serialize_into_with(out, &mut s, opts).expect("writing to NewlineCappedWriter never fails");
    let raw = s.into_string();
    if has_long_rule_line(&raw) {
        isolate_decorative_rules(&raw)
    } else {
        raw
    }
}

/// Serialize an [`LexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub fn serialize_into<W: Write>(out: &LexOutput, writer: &mut W) -> fmt::Result {
    serialize_into_with(out, writer, SerializeOptions::default())
}

/// Serialize an [`LexOutput`] into the given writer with explicit
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
pub fn serialize_into_with<W: Write>(
    out: &LexOutput,
    writer: &mut W,
    opts: SerializeOptions,
) -> fmt::Result {
    let mut tracking = TrackingWriter::new(writer);
    let mut sink = SerializeSink {
        store: &out.store,
        out: &mut tracking,
        directives: opts.directives,
    };
    walk(out, &mut sink)
}

/// [`WalkSink`] that re-emits Aozora source text from the AST,
/// threading the [`NodeStore`] (the resolve authority) into every AST emitter.
struct SerializeSink<'a, W: Write> {
    store: &'a NodeStore,
    out: &'a mut TrackingWriter<W>,
    /// Which notation-hygiene tiers to apply to `DirectiveKind::Unknown`
    /// near-misses (`Off` = verbatim; `Canonical` = Tier1; `Degraded` = Tier1+Tier2).
    directives: DirectiveNormalization,
}

impl<W: Write> WalkSink for SerializeSink<'_, W> {
    // Serialization copies `\n` verbatim, so it is not a structural event.
    const WANTS_NEWLINES: bool = false;

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.out.write_str(text)
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(n))
            | (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => {
                emit_aozora(n, self.store, self.out, self.directives)
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                emit_container_open(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                emit_container_close(close, self.out)
            }
            // Sentinel hit without a corresponding registry entry, or a
            // kind/variant mismatch — best-effort skip.
            _ => Ok(()),
        }
    }
}

/// Dispatch an owned [`Node`] to its per-variant source emitter.
fn emit_aozora<W: Write>(
    node: Node,
    store: &NodeStore,
    out: &mut TrackingWriter<W>,
    directives: DirectiveNormalization,
) -> fmt::Result {
    match node {
        Node::Ruby(r) => emit_ruby(&r, store, out),
        Node::Format(f) => emit_format(&f, store, out),
        Node::Gaiji(g) => emit_gaiji(&g, store, out),
        Node::Kaeriten(k) => emit_kaeriten(k, store, out),
        Node::Directive(a) => emit_annotation(a, store, out, directives),
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
// Content resolve layer — resolve and emit a `Content` / `ContentRange`,
// either verbatim or as plain text (gaiji written as its hint).
// ----------------------------------------------------------------------

/// Emit a single [`Content`] verbatim (plain text, or each segment's
/// text / gaiji / directive).
fn emit_content_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => out.write_str(store.resolve_str(id))?,
                    Segment::Gaiji(g) => emit_gaiji(&g, store, out)?,
                    Segment::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    // `Segment` is `#[non_exhaustive]`; forward-compat skip.
                    _ => {}
                }
            }
            Ok(())
        }
        // `Content` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

/// Emit a [`ContentRange`] run (length 1 by construction) by serializing each
/// resolved [`Content`].
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

/// Emit a single [`Content`] as plain text: a gaiji segment writes its
/// `hint`, not its glyph form.
fn emit_content_as_plain_one<W: Write>(c: Content, store: &NodeStore, out: &mut W) -> fmt::Result {
    match c {
        Content::Plain(id) => out.write_str(store.resolve_str(id)),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => out.write_str(store.resolve_str(id))?,
                    Segment::Gaiji(g) => out.write_str(store.resolve_str(g.hint))?,
                    Segment::Directive(a) => out.write_str(store.resolve_str(a.raw))?,
                    // `Segment` is `#[non_exhaustive]`; forward-compat skip.
                    _ => {}
                }
            }
            Ok(())
        }
        // `Content` is `#[non_exhaustive]`; forward-compat skip.
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
fn emit_ruby<W: Write>(r: &Ruby, store: &NodeStore, out: &mut TrackingWriter<W>) -> fmt::Result {
    if matches!(r.side, RubySide::Left) {
        emit_content_range(r.base, store, out)?;
        out.write_str("［＃「")?;
        emit_content_range(r.base, store, out)?;
        out.write_str("」の左に「")?;
        emit_content_range(r.reading, store, out)?;
        return out.write_str("」のルビ］");
    }
    if ruby_needs_bar(store.resolve_content_range(r.base), out.last(), store) {
        out.write_char('｜')?;
    }
    emit_content_range(r.base, store, out)?;
    out.write_char('《')?;
    emit_content_range(r.reading, store, out)?;
    out.write_char('》')
}

/// Decide whether a right-side ruby base needs an explicit `｜` start bar:
/// true when the base is not a uniform single `RubyBaseClass` run (a bare
/// reading would re-parse a shorter base), or the preceding char is the
/// same class as the base or `｜` (it would otherwise extend into the
/// base). A right-side base is always a single `Plain`; the resolved run is
/// matched accordingly. For a kanji base this is byte-for-byte the previous
/// `is_ruby_base_char`-based rule (the `Kanji` class equals the old set);
/// the class-awareness only governs the non-kanji bases that
/// `trailing_ruby_base_start` newly forms — the same lockstep the
/// classifier walks (ADR 0002).
fn ruby_needs_bar(base_run: &[Content], prev: Option<char>, store: &NodeStore) -> bool {
    // An all-gaiji base (`※［＃…］《…》` or an adjacent run `※…※…《…》`) re-parses
    // implicitly via the classifier's deferred-emit accumulation, so it never
    // needs an explicit `｜` — a preceding character cannot extend into a
    // structured gaiji node, and adjacent gaiji re-accumulate into one base.
    // Emitting a bar here would inject a `｜` absent from the source and break
    // the round-trip fixed point.
    if let [Content::Segments(range)] = base_run {
        let segs = store.resolve_seg_range(*range);
        if !segs.is_empty() && segs.iter().all(|s| matches!(s, Segment::Gaiji(_))) {
            return false;
        }
    }
    let plain = match base_run {
        [Content::Plain(id)] => Some(store.resolve_str(*id)),
        _ => None,
    };
    plain.is_none_or(|s| {
        let Some(base_class) = s.chars().next_back().and_then(ruby_base_class) else {
            return true;
        };
        s.chars().any(|c| ruby_base_class(c) != Some(base_class))
            || prev.is_some_and(|c| ruby_base_class(c) == Some(base_class) || c == '｜')
    })
}

/// Serialize a forward-format node to its `［＃…］` bracket form. A `Reclaimed`
/// origin first re-emits the target literal; a bouten attribute uses the
/// `…に<keyword>` (or `の左に`) shape; a font-size attribute spells its
/// `N段階大きな/小さな文字` magnitude; every other attribute uses
/// `［＃「target」は<keyword>］`.
fn emit_format<W: Write>(f: &ForwardFormat, store: &NodeStore, out: &mut W) -> fmt::Result {
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
        emit_bouten_targets(store.resolve_content_range(f.target), store, out)?;
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
        // Anchor is not distinguished in the model (like LineFormat::AlignEnd), so
        // canonicalise: 0 → 地付き (the zero-lift spelling), else 文末より…字上げ揃え.
        // Both re-parse to the same offset.
        if offset == 0 {
            out.write_str("地付き")?;
        } else {
            write!(out, "文末より{offset}字上げ揃え")?;
        }
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

/// Serialize a gaiji node to its `［＃「hint」…］` bracket form (prefixed with
/// `※` unless `standalone`), appending the mencode tail when the canonical
/// form carries one.
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
/// `Unresolved` tail's [`StrId`](aozora_syntax::ast::StrId) against `store`.
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
fn emit_kaeriten<W: Write>(k: Kaeriten, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(store.resolve_str(k.mark))?;
    out.write_char('］')
}

/// Serialize a directive by writing its `raw` bytes verbatim (they already
/// include the `［＃…］` brackets).
///
/// When `directives` is not `Off`, an `Unknown` directive whose trimmed body is
/// a verified near-miss is rewritten to `［＃<canonical>］` instead. Tier1
/// ([`canonical_directive`]) is tried first; at the `Degraded` level a Tier1
/// miss then tries the lossy / judgment Tier2 reductions ([`degraded_directive`]).
/// The rewrite is idempotent: the resolved body parses to a recognized
/// (non-`Unknown`) node, so a second pass never re-enters this branch. Tier2 is
/// reached only from the ephemeral `Degraded` render buffer, never from a
/// persistent-write path — so a Tier2 reduction never rewrites source.
fn emit_annotation<W: Write>(
    a: Directive,
    store: &NodeStore,
    out: &mut W,
    directives: DirectiveNormalization,
) -> fmt::Result {
    let raw = store.resolve_str(a.raw);
    if directives != DirectiveNormalization::Off && a.kind == DirectiveKind::Unknown {
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

/// Serialize an angle-quote to its `≪…≫` form.
fn emit_angle_quote<W: Write>(d: AngleQuote, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_char('≪')?;
    emit_content_range(d.content, store, out)?;
    out.write_char('≫')
}

/// Serialize a margin note to its `base［＃「base」…］` form, using the kind's
/// connector / suffix affixes around the note text.
fn emit_side_note<W: Write>(s: &MarginNote, store: &NodeStore, out: &mut W) -> fmt::Result {
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
fn emit_sashie<W: Write>(s: &Illustration, store: &NodeStore, out: &mut W) -> fmt::Result {
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
fn emit_heading_hint<W: Write>(h: HeadingHint, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("［＃「")?;
    out.write_str(store.resolve_str(h.target))?;
    out.write_str("」は")?;
    out.write_str(heading_style_keyword(h.style))?;
    out.write_str(heading_level_word(h.level))?;
    out.write_str("］")
}

/// Serialize an Aozora heading to its referent line followed by the
/// `［＃「text」は<style><level>見出し］` bracket.
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

    use aozora_syntax::alloc::Allocator;
    use aozora_syntax::{
        BoutenKind, HeadingKind, HeadingStyle, MarginNoteKind, RegionClose, RegionFormat,
    };

    /// `serialize ∘ parse` reaches a fixed point after one pass — the
    /// canonical round-trip contract (end-to-end byte-identity is pinned by the
    /// conformance golden).
    fn assert_parity(src: &str) {
        let first = serialize(&aozora_pipeline::lex(src));
        let second = serialize(&aozora_pipeline::lex(&first));
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
        use aozora_syntax::alloc::Allocator;
        use aozora_syntax::ast::Node;
        use aozora_syntax::{ForwardAttr, ForwardOrigin};

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

    /// E1-4: a self-contained heading hint serializes to the bracket alone — it
    /// must NOT fabricate a referent line, which would re-parse as a promotable
    /// heading (`promote_headings`) and break the round-trip fixed point.
    #[test]
    fn self_contained_heading_serializes_bracket_only() {
        use aozora_syntax::alloc::Allocator;
        use aozora_syntax::ast::Node;
        use aozora_syntax::{HeadingKind, HeadingStyle};

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

    // ------------------------------------------------------------------
    // Direct-emitter mutation kills — each test pins the exact serialized
    // bytes / boolean decision an emitter produces for a representative
    // input, so a stubbed body / swapped operator / deleted arm diverges.
    // ------------------------------------------------------------------

    /// Run `emit_aozora` for a unit-leaf node through a `TrackingWriter` and
    /// return the emitted source.
    fn emit_via_tracking(node: Node, store: &NodeStore) -> String {
        let mut buf = String::new();
        let mut tw = TrackingWriter::new(&mut buf);
        emit_aozora(node, store, &mut tw, DirectiveNormalization::Off)
            .expect("serialize into String is infallible");
        buf
    }

    /// Drive one `SerializeSink::on_node` dispatch and return the emitted
    /// source, so the container-open / container-close arms are exercised.
    fn on_node_via_sink(store: &NodeStore, kind: SentinelKind, node: NodeRef) -> String {
        let mut buf = String::new();
        let mut tw = TrackingWriter::new(&mut buf);
        let mut sink = SerializeSink {
            store,
            out: &mut tw,
            directives: DirectiveNormalization::Off,
        };
        sink.on_node(kind, node)
            .expect("serialize into String is infallible");
        buf
    }

    /// Serialize a single generic directive with the given normalization tier.
    fn annotate(raw: &str, kind: DirectiveKind, directives: DirectiveNormalization) -> String {
        let mut a = Allocator::new();
        let d = a.make_directive(raw, kind);
        let store = a.into_store();
        let mut buf = String::new();
        emit_annotation(d, &store, &mut buf, directives)
            .expect("serialize into String is infallible");
        buf
    }

    /// `serialize_into` must write the actually-walked source, not a stubbed
    /// empty `Ok(())`.
    #[test]
    fn serialize_into_emits_the_walked_source() {
        let out = aozora_pipeline::lex("あ");
        let mut buf = String::new();
        serialize_into(&out, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "あ");
    }

    /// The `on_node` container-open / container-close arms must dispatch to the
    /// marker spellers, not fall through to the `_ => Ok(())` skip.
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

    /// The unit-leaf `emit_aozora` arms each spell their own directive.
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

    /// `emit_content_one` writes a gaiji segment via its `※［＃…］` bracket form.
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

    /// `emit_content_as_plain_one` walks a `Segments` content and writes each
    /// arm — text verbatim, gaiji as its `hint`, directive as its raw bytes.
    #[test]
    fn emit_content_as_plain_one_writes_every_segment() {
        let mut a = Allocator::new();
        let text = a.seg_text("あ");
        let gaiji = a.make_gaiji("げ", None, false);
        let gseg = a.seg_gaiji(gaiji);
        let directive = a.make_directive("［＃注記］", DirectiveKind::Unknown);
        let dseg = a.seg_annotation(directive);
        let content = a.content_segments(&[text, gseg, dseg]);
        let store = a.into_store();
        let mut buf = String::new();
        emit_content_as_plain_one(content, &store, &mut buf)
            .expect("serialize into String is infallible");
        assert_eq!(buf, "あげ［＃注記］");
    }

    /// `ruby_needs_bar` at every boundary: a uniform single-class base and an
    /// all-gaiji base both decline the explicit `｜`; a mixed (non-uniform)
    /// base demands it.
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
        let store = a.into_store();

        assert!(!ruby_needs_bar(&[uniform], None, &store));
        assert!(!ruby_needs_bar(&[all_gaiji], None, &store));
        assert!(ruby_needs_bar(&[mixed], None, &store));

        // The predecessor clause (line 360) must be exercised deterministically,
        // not left to the property suite: a SAME-class predecessor forces the bar
        // even for a uniform base (kills `|| -> &&` and the `class == base`
        // `== -> !=`), a bar predecessor forces it via the `c == '｜'` arm, and a
        // DIFFERENT-class predecessor must not (the clause stays false).
        assert!(ruby_needs_bar(&[uniform], Some('一'), &store));
        assert!(ruby_needs_bar(&[uniform], Some('｜'), &store));
        assert!(!ruby_needs_bar(&[uniform], Some('a'), &store));
    }

    /// The bouten position keyword: `の左に` / `の両側に` / bare `に`.
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

    /// The forward `AlignEnd` offset boundary: `0` → 地付き, `N` → 文末より…揃え.
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

    /// Each accent suffix spells its bracketed mark symbol exactly.
    #[test]
    fn accent_suffix_exact() {
        assert_eq!(accent_suffix(AccentMark::Acute), "アクサン（´）付き");
        assert_eq!(accent_suffix(AccentMark::Grave), "アクサン（｀）付き");
        assert_eq!(accent_suffix(AccentMark::Umlaut), "ウムラウト（¨）付き");
    }

    /// A single-`Plain` bouten target becomes one quoted `「…」` run.
    #[test]
    fn emit_bouten_targets_plain_run() {
        let mut a = Allocator::new();
        let c = a.content_plain("甲");
        let store = a.into_store();
        let mut buf = String::new();
        emit_bouten_targets(&[c], &store, &mut buf).expect("serialize into String is infallible");
        assert_eq!(buf, "「甲」");
    }

    /// A segmented bouten target splits its text on `、` into one `「part」`
    /// per non-empty piece (and emits nothing spurious when all parts are
    /// present).
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

    /// `gaiji_has_mencode` is false only for the mencode-less `Unresolved`.
    #[test]
    fn gaiji_has_mencode_reflects_mencode_presence() {
        assert!(!gaiji_has_mencode(GaijiCanonicalOwned::Unresolved {
            mencode: None
        }));
        assert!(gaiji_has_mencode(GaijiCanonicalOwned::Unicode('あ')));
    }

    /// A kaeriten mark round-trips to its `［＃<mark>］` bracket.
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

    /// The directive-normalization gate: a Tier1 near-miss is rewritten only
    /// for an `Unknown` directive at a non-`Off` tier; a Tier2 degraded body is
    /// rewritten only at the `Degraded` tier.
    #[test]
    fn emit_annotation_directive_normalization() {
        assert_eq!(
            annotate(
                "［＃ゴチック］",
                DirectiveKind::Unknown,
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
                DirectiveKind::Unknown,
                DirectiveNormalization::Off,
            ),
            "［＃ゴチック］",
        );
        assert_eq!(
            annotate(
                "［＃ここから最後まで3字下げ］",
                DirectiveKind::Unknown,
                DirectiveNormalization::Degraded,
            ),
            "［＃ここから3字下げ］",
        );
        assert_eq!(
            annotate(
                "［＃ここから最後まで3字下げ］",
                DirectiveKind::Unknown,
                DirectiveNormalization::Canonical,
            ),
            "［＃ここから最後まで3字下げ］",
        );
    }

    /// A gloss margin note round-trips its base / connector / note / suffix.
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

    /// A keyword-form illustration round-trips its 挿絵 / number / file /
    /// dimensions.
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

    /// An Aozora heading round-trips its referent line plus the level bracket.
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
