//! Aozora text serialiser.
//!
//! Inverse of the `aozora-lexer` pipeline: consumes the normalized
//! text and placeholder registry captured on [`crate::ParseResult`]
//! and rebuilds the Aozora-format source by **substituting each PUA
//! sentinel back into its original Aozora markup**. Plain text
//! between sentinels passes through verbatim — the lexer's
//! normalization step preserves anything it did not classify as
//! Aozora notation.
//!
//! ## Why byte walk, not tree walk
//!
//! The lexer already produced `normalized` as a string with Aozora
//! spans reduced to 1-character PUA sentinels and everything else
//! passed through verbatim, so the inverse is trivial:
//!
//! 1. Walk `normalized` byte-by-byte (char-by-char for UTF-8).
//! 2. At each sentinel `U+E001..=U+E004`, look the position up in
//!    the registry and emit the Aozora markup for the stored node.
//! 3. Everything else passes through.
//!
//! Runtime is `O(normalized.len())` with one binary-search probe
//! per sentinel (via the registry's existing accessors). No tree
//! traversal, no per-variant re-escaping.
//!
//! ## Round-trip stability
//!
//! The emitted markup is *canonical*, not byte-identical with the
//! source:
//!
//! * Ruby always renders as explicit `｜BASE《READING》` even when
//!   the source used the implicit trailing-kanji form — both parse
//!   to the same AST, so the canonicalisation stays stable.
//! * Bouten with multi-target [`Content::Segments`] emits each
//!   target as `「X」` separately (mirrors the lexer's multi-quote
//!   recogniser).
//! * Gaiji emits `mencode` when present; description-only forms
//!   omit the tail.
//! * Containers keep their paired `［＃ここから…］` / `［＃ここで…終わり］`
//!   bracket pair. The emitted shape is whatever the lexer expects
//!   to re-classify back into the same `ContainerKind`, so the
//!   second serialise is byte-identical to the first.
//!
//! The corpus sweep's `I3` invariant hard-gates this contract:
//! `serialize ∘ parse` is a fixed point after one round-trip.

use aozora_lexer::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::{
    AlignEnd, Annotation, AozoraNode, Bouten, BoutenKind, BoutenPosition, ContainerKind, Content,
    DoubleRuby, Gaiji, HeadingHint, Indent, Kaeriten, Ruby, Sashie, SectionKind, SegmentRef,
    TateChuYoko,
};

use crate::{ParseArtifacts, ParseResult};

/// Serialize a parsed document back into Aozora-format source text.
///
/// Convenience wrapper over [`serialize_from_artifacts`].
///
/// # Complexity
///
/// `O(normalized.len() + sentinels * log(registry.len()))`. The
/// registry access is the `phase5_registry` binary-search accessor;
/// the outer walk is a single forward pass over the normalized text.
#[must_use]
pub fn serialize(result: &ParseResult) -> String {
    serialize_from_artifacts(&result.artifacts)
}

/// Sentinel dispatch. Returning an enum here (instead of branching
/// inside the outer loop per-variant) keeps the hot path flat and
/// the emitter table open for future sentinel kinds without another
/// `match` rewrite.
enum SentinelKind {
    Inline,
    BlockLeaf,
    BlockOpen,
    BlockClose,
}

impl SentinelKind {
    const fn from_char(c: char) -> Option<Self> {
        match c {
            INLINE_SENTINEL => Some(Self::Inline),
            BLOCK_LEAF_SENTINEL => Some(Self::BlockLeaf),
            BLOCK_OPEN_SENTINEL => Some(Self::BlockOpen),
            BLOCK_CLOSE_SENTINEL => Some(Self::BlockClose),
            _ => None,
        }
    }
}

/// Serialize directly from a [`ParseArtifacts`] bundle.
///
/// Useful when a downstream caller only retains the artifacts
/// (e.g. an LSP backend that holds onto registry + normalized text
/// across edits without keeping the full [`ParseResult`] alive).
///
/// # Panics
///
/// Does not panic on well-formed input. The lexer's Phase 0 caps
/// `normalized.len()` at `u32::MAX`, which guarantees every
/// `match_indices` position fits in `u32`; if that invariant were
/// ever violated upstream, the index conversion would panic.
#[must_use]
pub fn serialize_from_artifacts(artifacts: &ParseArtifacts) -> String {
    let normalized = &artifacts.normalized;
    let registry = &artifacts.registry;

    // `match_indices` with a predicate gives us every sentinel's
    // byte position in one linear sweep (internally memchr-like).
    // Between hits we bulk-copy the plain chunk via `push_str`
    // instead of char-by-char — big win on Japanese text where
    // every char is 3 bytes, and keeps the O(n) walk branch-free
    // in its fast path.
    let mut out = NewlineCappedWriter::with_capacity(normalized.len() * 2);

    let mut cursor = 0usize;
    for (pos, sentinel_str) in
        normalized.match_indices(|c: char| SentinelKind::from_char(c).is_some())
    {
        // Flush the plain chunk between the previous cursor and
        // this sentinel. `push_str` is one memcpy plus the
        // newline-cap state update amortised across the chunk.
        out.push_str(&normalized[cursor..pos]);

        let sentinel_ch = sentinel_str
            .chars()
            .next()
            .expect("match_indices yields non-empty match");
        let byte_pos = u32::try_from(pos).expect("normalized fits u32 per Phase 0 cap");
        // Dispatch via the registry accessor for this sentinel
        // class. Drift (no registry entry) silently drops the
        // sentinel; Phase 6's V2/V3 diagnostics would have already
        // flagged the shape.
        match SentinelKind::from_char(sentinel_ch).expect("predicate matched on this char") {
            SentinelKind::Inline => {
                if let Some(node) = registry.inline_at(byte_pos) {
                    let mut buf = String::new();
                    emit_aozora(node, &mut buf);
                    out.push_str(&buf);
                }
            }
            SentinelKind::BlockLeaf => {
                if let Some(node) = registry.block_leaf_at(byte_pos) {
                    let mut buf = String::new();
                    emit_aozora(node, &mut buf);
                    out.push_str(&buf);
                }
            }
            SentinelKind::BlockOpen => {
                if let Some(kind) = registry.block_open_at(byte_pos) {
                    out.push_str(container_open_marker(kind));
                }
            }
            SentinelKind::BlockClose => {
                if let Some(kind) = registry.block_close_at(byte_pos) {
                    out.push_str(container_close_marker(kind));
                }
            }
        }
        cursor = pos + sentinel_str.len();
    }
    // Flush the tail.
    out.push_str(&normalized[cursor..]);
    out.into_string()
}

/// Output buffer that caps consecutive `\n` runs at two on-the-fly.
///
/// Phase 4 of the lexer pads every block sentinel with `\n\n`
/// unconditionally, so naively round-tripping the serializer's
/// output back through parse inflates the blank-line run by two
/// per iteration. Capping at 2 here makes `serialize ∘ parse` a
/// fixed point after the first pass (CommonMark folds any run of
/// ≥2 newlines to a single paragraph break, so the cap is
/// semantics-preserving).
///
/// Single-pass inline cap beats a post-pass sweep by avoiding a
/// second O(n) traversal — important for 2-MB corpus items.
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

    fn push_str(&mut self, s: &str) {
        // Fast path: no newline in the chunk — one memcpy and reset
        // the trailing-newline counter. memchr hot path.
        if !s.contains('\n') {
            if !s.is_empty() {
                self.out.push_str(s);
                self.trailing_newlines = 0;
            }
            return;
        }
        // Slow path: chunk contains newlines. Walk char-by-char so
        // the cap is honoured at every position. UTF-8 char iteration
        // is cheap relative to the memcpy it replaces only on chunks
        // dense with newlines.
        for ch in s.chars() {
            if ch == '\n' {
                self.trailing_newlines += 1;
                if self.trailing_newlines <= 2 {
                    self.out.push('\n');
                }
            } else {
                self.trailing_newlines = 0;
                self.out.push(ch);
            }
        }
    }

    fn into_string(self) -> String {
        self.out
    }
}

/// Append the afm markup for an [`AozoraNode`] to `out`. Covers all
/// variants currently produced by the lexer; unknown variants emit
/// a visible `<!-- unsupported-aozora: … -->` placeholder so a
/// round-trip gap is diagnosable rather than silent.
fn emit_aozora(node: &AozoraNode, out: &mut String) {
    match node {
        AozoraNode::Ruby(r) => emit_ruby(r, out),
        AozoraNode::Bouten(b) => emit_bouten(b, out),
        AozoraNode::TateChuYoko(t) => emit_tate_chu_yoko(t, out),
        AozoraNode::Gaiji(g) => emit_gaiji(g, out),
        AozoraNode::Kaeriten(k) => emit_kaeriten(k, out),
        AozoraNode::Annotation(a) => emit_annotation(a, out),
        AozoraNode::DoubleRuby(d) => emit_double_ruby(d, out),
        AozoraNode::PageBreak => out.push_str("［＃改ページ］"),
        AozoraNode::SectionBreak(kind) => emit_section_break(*kind, out),
        AozoraNode::Indent(i) => emit_indent(*i, out),
        AozoraNode::AlignEnd(a) => emit_align_end(*a, out),
        AozoraNode::Sashie(s) => emit_sashie(s, out),
        AozoraNode::HeadingHint(h) => emit_heading_hint(h, out),
        _ => {
            // Aozora variants the serialiser doesn't yet cover
            // (`Container` is handled by the open/close sentinel
            // path, never by inline/block-leaf; `Warichu`,
            // `Keigakomi`, `AozoraHeading` land here).
            out.push_str("<!-- unsupported-aozora: ");
            out.push_str(node.xml_node_name());
            out.push_str(" -->");
        }
    }
}

fn emit_ruby(r: &Ruby, out: &mut String) {
    out.push('｜');
    emit_content(&r.base, out);
    out.push('《');
    emit_content(&r.reading, out);
    out.push('》');
}

fn emit_bouten(b: &Bouten, out: &mut String) {
    // The PUA sentinel only spans `［＃「…」(particle)(kind)］` — the
    // preceding target text is plain bytes already copied through
    // by the outer walk, so we must NOT re-emit it here.
    out.push_str("［＃");
    emit_bouten_targets(&b.target, out);
    match b.position {
        BoutenPosition::Left => out.push_str("の左に"),
        _ => out.push('に'),
    }
    out.push_str(bouten_kind_keyword(b.kind));
    out.push('］');
}

/// Render each target run as a separate `「X」` chunk so the next
/// parse's multi-quote classifier picks them back up as the same
/// Segments shape.
fn emit_bouten_targets(c: &Content, out: &mut String) {
    match c {
        Content::Plain(s) => {
            out.push('「');
            out.push_str(s);
            out.push('」');
        }
        Content::Segments(segs) => {
            let mut any = false;
            for seg in segs {
                if let aozora_syntax::Segment::Text(t) = seg
                    && !t.is_empty()
                {
                    // Emit each comma-separated chunk as its own
                    // `「…」`. The classifier joins multi-targets
                    // with `、`, so re-splitting on `、` recovers the
                    // original target list.
                    for part in t.split('、').filter(|p| !p.is_empty()) {
                        out.push('「');
                        out.push_str(part);
                        out.push('」');
                        any = true;
                    }
                }
            }
            if !any {
                // Defensive fallback: empty or Segments-only-with-
                // non-Text content still needs *some* target shape
                // so the next parse doesn't reclassify as an unknown
                // annotation. Use an empty quote pair.
                out.push('「');
                out.push('」');
            }
        }
        _ => {}
    }
}

fn emit_tate_chu_yoko(t: &TateChuYoko, out: &mut String) {
    // Same rule as bouten — the sentinel spans only the
    // `［＃「…」は縦中横］` annotation, not the target text.
    out.push_str("［＃「");
    emit_content_as_plain(&t.text, out);
    out.push_str("」は縦中横］");
}

fn emit_gaiji(g: &Gaiji, out: &mut String) {
    // Aozora gaiji shape is always `※［＃「description」、mencode］` or
    // `※［＃「description」］`. The closing 」 was previously omitted,
    // which `round_trip_fixed_point` could not catch (the malformed
    // form still re-parses to the same node so the second-pass output
    // equals the first). The byte-identity test
    // `gaiji_with_mencode_emits_canonical_shape_on_first_serialize`
    // locks the shape so the regression cannot land silently again.
    out.push('※');
    out.push_str("［＃「");
    out.push_str(&g.description);
    out.push('」');
    if let Some(m) = &g.mencode {
        out.push('、');
        out.push_str(m);
    }
    out.push('］');
}

fn emit_kaeriten(k: &Kaeriten, out: &mut String) {
    out.push_str("［＃");
    out.push_str(&k.mark);
    out.push('］');
}

fn emit_annotation(a: &Annotation, out: &mut String) {
    // `raw` is the verbatim `［＃…］` byte range from the source;
    // round-tripping is byte-identical.
    out.push_str(&a.raw);
}

fn emit_double_ruby(d: &DoubleRuby, out: &mut String) {
    out.push('《');
    out.push('《');
    emit_content(&d.content, out);
    out.push('》');
    out.push('》');
}

fn emit_section_break(kind: SectionKind, out: &mut String) {
    let keyword = match kind {
        SectionKind::Choho => "改丁",
        SectionKind::Dan => "改段",
        SectionKind::Spread => "改見開き",
        _ => "改ページ",
    };
    out.push_str("［＃");
    out.push_str(keyword);
    out.push('］');
}

fn emit_indent(i: Indent, out: &mut String) {
    use core::fmt::Write as _;
    // Width 1 is the "default" indent marker grammar in the spec
    // (`［＃字下げ］` is implicitly "1"). For ≥2 we keep the digit.
    if i.amount == 1 {
        out.push_str("［＃字下げ］");
    } else {
        write!(out, "［＃{}字下げ］", i.amount).expect("writing to a String is infallible");
    }
}

fn emit_align_end(a: AlignEnd, out: &mut String) {
    use core::fmt::Write as _;
    if a.offset == 0 {
        out.push_str("［＃地付き］");
    } else {
        write!(out, "［＃地から{}字上げ］", a.offset).expect("writing to a String is infallible");
    }
}

fn emit_sashie(s: &Sashie, out: &mut String) {
    out.push_str("［＃挿絵（");
    out.push_str(&s.file);
    out.push_str("）入る］");
}

/// Serialise a heading hint back to `［＃「target」は(大|中|小)見出し］`.
///
/// `post_process::splice_heading_hint` removes the hint from the AST
/// by promoting its host paragraph to a Markdown heading; the
/// serialiser still walks the lexer's `PlaceholderRegistry`, so the
/// original markup is reconstructed from the hint fields regardless
/// of AST-side promotion. Keeping the two in sync is what makes I3
/// (`serialize ∘ parse` fixed point) hold for heading-hint paragraphs.
///
/// Level 1 / 2 / 3 cover the three spec keywords; the classifier
/// rejects any other level, so the `_` arm in the match is
/// unreachable for inputs that originated from the lexer. We keep it
/// — rather than `unreachable!()` — to avoid a panic path on any
/// future construction of `HeadingHint` from non-lexer callers; a
/// best-effort fallback to `小見出し` keeps the round-trip string
/// well-formed.
fn emit_heading_hint(h: &HeadingHint, out: &mut String) {
    out.push_str("［＃「");
    out.push_str(&h.target);
    out.push_str(match h.level {
        1 => "」は大見出し］",
        2 => "」は中見出し］",
        _ => "」は小見出し］",
    });
}

fn container_open_marker(kind: ContainerKind) -> &'static str {
    // The lexer's classify_container_open / close grammar is
    // narrowly defined; the canonical markers it accepts are what we
    // emit, so re-parse hits the same `ContainerKind`. The `_` arm
    // subsumes `Indent { .. }` (the dominant case) plus any future
    // `#[non_exhaustive]` variant; falling back to 字下げ keeps the
    // bracket well-formed regardless.
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここから地付き］",
        ContainerKind::Keigakomi => "［＃罫囲み］",
        ContainerKind::Warichu => "［＃割り注］",
        _ => "［＃ここから字下げ］",
    }
}

fn container_close_marker(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::AlignEnd { .. } => "［＃ここで地付き終わり］",
        ContainerKind::Keigakomi => "［＃罫囲み終わり］",
        ContainerKind::Warichu => "［＃割り注終わり］",
        _ => "［＃ここで字下げ終わり］",
    }
}

fn bouten_kind_keyword(kind: BoutenKind) -> &'static str {
    // `Goma` is the default bouten shape; the `_` arm covers it plus
    // any future #[non_exhaustive] additions and returns the bare
    // `傍点` keyword the classifier treats as the default kind.
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

fn emit_content(c: &Content, out: &mut String) {
    for seg in c {
        match seg {
            SegmentRef::Text(t) => out.push_str(t),
            SegmentRef::Gaiji(g) => emit_gaiji(g, out),
            SegmentRef::Annotation(a) => emit_annotation(a, out),
            _ => {}
        }
    }
}

/// Flatten [`Content`] to its plain textual form, discarding any
/// embedded Gaiji/Annotation wrapping. Used for `「…」` contexts
/// where the target reference must be a single text literal.
fn emit_content_as_plain(c: &Content, out: &mut String) {
    for seg in c {
        match seg {
            SegmentRef::Text(t) => out.push_str(t),
            SegmentRef::Gaiji(g) => out.push_str(&g.description),
            SegmentRef::Annotation(a) => out.push_str(&a.raw),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// Parse once, serialize, assert the output is non-empty and
    /// looks canonical.
    fn once(src: &str) -> String {
        serialize(&parse(src))
    }

    /// Parse, serialize, parse again, serialize — the two outputs
    /// must match byte-for-byte. This is the corpus sweep's I3
    /// invariant in unit-test form.
    fn round_trip_fixed_point(src: &str) {
        let first = once(src);
        let second = once(&first);
        assert_eq!(first, second, "serialize ∘ parse is not a fixed point");
    }

    #[test]
    fn plain_paragraph_round_trips_exactly() {
        let out = once("hello world");
        assert_eq!(out, "hello world");
        round_trip_fixed_point("hello world");
    }

    #[test]
    fn explicit_ruby_round_trips() {
        round_trip_fixed_point("｜青梅《おうめ》");
        assert!(once("｜青梅《おうめ》").contains("｜青梅《おうめ》"));
    }

    #[test]
    fn implicit_ruby_canonicalises_to_explicit() {
        // First-pass output: `｜漢字《かんじ》` (explicit form).
        let first = once("漢字《かんじ》");
        assert!(
            first.contains("｜漢字《かんじ》"),
            "first pass must emit explicit form, got {first:?}"
        );
        // Second pass: stable.
        round_trip_fixed_point(&first);
    }

    #[test]
    fn forward_bouten_round_trips_with_kind_and_particle() {
        round_trip_fixed_point("可哀想［＃「可哀想」に傍点］という気");
    }

    #[test]
    fn forward_bouten_left_position_round_trips() {
        round_trip_fixed_point("X［＃「X」の左に傍点］");
    }

    #[test]
    fn gaiji_with_mencode_round_trips() {
        round_trip_fixed_point(
            "※[＃「木＋吶のつくり」、第3水準9-99-99］"
                .replace('[', "［")
                .as_str(),
        );
        round_trip_fixed_point("※［＃「木＋吶のつくり」、第3水準9-99-99］");
    }

    #[test]
    fn gaiji_with_mencode_emits_canonical_shape_on_first_serialize() {
        // Byte-identity on the *first* pass — the fixed-point test
        // above only catches stability, not correctness, so a
        // serializer that silently drops the closing 」 still passes
        // `gaiji_with_mencode_round_trips`. This test pins the
        // shape that actually matches the Aozora annotation manual.
        let src = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let out = once(src);
        assert_eq!(
            out, src,
            "first-pass serializer must preserve closing 」; got {out:?}",
        );
    }

    #[test]
    fn gaiji_without_mencode_emits_canonical_shape_on_first_serialize() {
        // description-only gaiji (no `、mencode` tail) still needs
        // the closing 」 on first pass.
        let src = "※［＃「〓」］";
        let out = once(src);
        assert_eq!(
            out, src,
            "first-pass serializer must preserve closing 」 even without mencode; got {out:?}",
        );
    }

    #[test]
    fn page_break_round_trips() {
        round_trip_fixed_point("前\n\n［＃改ページ］\n\n後");
    }

    #[test]
    fn section_break_round_trips() {
        round_trip_fixed_point("前\n\n［＃改丁］\n\n後");
    }

    #[test]
    fn paired_indent_container_round_trips() {
        round_trip_fixed_point("［＃ここから字下げ］\n本文\n［＃ここで字下げ終わり］");
    }

    #[test]
    fn paired_keigakomi_container_round_trips() {
        round_trip_fixed_point("［＃罫囲み］\n引用\n［＃罫囲み終わり］");
    }

    #[test]
    fn kaeriten_round_trips() {
        round_trip_fixed_point("学而時習之［＃一］");
    }

    #[test]
    fn double_ruby_round_trips() {
        round_trip_fixed_point("前《《強調》》後");
    }

    #[test]
    fn unknown_annotation_passes_through_verbatim() {
        let out = once("前［＃未知の注記］後");
        assert!(out.contains("［＃未知の注記］"), "got: {out:?}");
        round_trip_fixed_point("前［＃未知の注記］後");
    }

    #[test]
    fn mixed_inline_ruby_and_bouten_round_trip() {
        round_trip_fixed_point("｜青梅《おうめ》の地で可哀想［＃「可哀想」に傍点］と彼は言った");
    }

    #[test]
    fn serialize_on_empty_input_is_empty() {
        let out = once("");
        assert_eq!(out, "");
    }

    #[test]
    fn tate_chu_yoko_round_trips() {
        round_trip_fixed_point("25［＃「25」は縦中横］");
    }

    #[test]
    fn multi_quote_bouten_round_trips() {
        round_trip_fixed_point("AとB［＃「A」「B」に傍点］");
    }

    #[test]
    fn indent_align_end_leaf_round_trip() {
        round_trip_fixed_point("［＃地付き］");
        round_trip_fixed_point("［＃地から2字上げ］");
    }

    // ---------------------------------------------------------------
    // Property: `serialize ∘ parse` is a fixed point after one
    // round-trip for any afm-shaped input the Aozora pipeline
    // accepts. Generator is Aozora-trigger-rich so each shrunk
    // counterexample pins an actionable bug.
    // ---------------------------------------------------------------

    use proptest::prelude::*;

    fn aozora_char() -> impl Strategy<Value = char> {
        prop_oneof![
            Just('a'),
            Just('あ'),
            Just('漢'),
            Just('字'),
            Just('可'),
            Just('哀'),
            Just('想'),
            Just('｜'),
            Just('《'),
            Just('》'),
            Just('［'),
            Just('］'),
            Just('＃'),
            Just('※'),
            Just('「'),
            Just('」'),
            Just('、'),
            Just('に'),
            Just('は'),
            Just('傍'),
            Just('点'),
            Just(' '),
            Just('\n'),
        ]
    }

    fn aozora_fragment() -> impl Strategy<Value = String> {
        prop::collection::vec(aozora_char(), 0..40).prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        /// `serialize(parse(serialize(parse(src))))` must equal
        /// `serialize(parse(src))` for every generated shape — one
        /// round-trip fixes the normalisation, two must coincide.
        #[test]
        fn serialize_is_a_fixed_point(src in aozora_fragment()) {
            let first = once(&src);
            let second = once(&first);
            prop_assert_eq!(first, second);
        }

        /// Newline runs of 3+ must never appear in serialiser output.
        /// This is the semantic contract of `NewlineCappedWriter`;
        /// regressions break the fixed-point property above.
        #[test]
        fn output_never_contains_three_consecutive_newlines(src in aozora_fragment()) {
            let out = once(&src);
            prop_assert!(
                !out.contains("\n\n\n"),
                "serializer emitted ≥3 consecutive newlines: {out:?}",
            );
        }

        /// No PUA sentinel character (U+E001..=U+E004) may survive
        /// to the serializer's output. If one does, the registry
        /// substitution missed — a real bug.
        #[test]
        fn output_never_contains_pua_sentinels(src in aozora_fragment()) {
            let out = once(&src);
            for sentinel in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
                prop_assert!(
                    !out.contains(sentinel),
                    "serializer leaked PUA sentinel U+{:04X}: {out:?}",
                    sentinel as u32,
                );
            }
        }
    }
}
