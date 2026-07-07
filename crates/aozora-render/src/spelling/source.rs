//! Lifetime-free Aozora-source serialize helpers.
//!
//! The shared, AST-payload-free marker emitters the owned serializer
//! (`serialize`) and the source splice (#202) reuse: the
//! container open/close marker spellers, the single-line layout-directive
//! emitter, the heading keyword helpers, the `TrackingWriter` (tracks the last
//! emitted char for the bare-`｜` decision), and the `NewlineCappedWriter`
//! (caps the block-padding blank-line run so `serialize ∘ parse` is a fixed
//! point). Every function takes only `Copy` scalar payloads, so the byte
//! spelling is single-source.

use core::fmt::{self, Write};

use aozora_syntax::{
    BlockStyles, BoutenPosition, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, LineFormat,
    RegionClose, RegionFormat, SectionKind,
};

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
    /// Wrap `inner`, with no predecessor char recorded yet. The construction
    /// site for the owned serializer's tracking writer.
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

pub(crate) fn emit_section_break<W: Write>(kind: SectionKind, out: &mut W) -> fmt::Result {
    out.write_str("［＃")?;
    out.write_str(kind.keyword())?;
    out.write_char('］')
}

pub(crate) fn emit_line<W: Write>(lf: LineFormat, out: &mut W) -> fmt::Result {
    match lf {
        // Both-margin compound: a head indent plus a foot-edge lift. `字あき`
        // is canonicalised to the `地よりM字上げで` spelling (same foot-edge
        // semantics; the `て`/bare-join input variants converge here too).
        LineFormat::Indent {
            amount,
            end_offset: Some(offset),
        } => write!(out, "［＃{amount}字下げ、地より{offset}字上げで］"),
        LineFormat::Indent {
            amount: 1,
            end_offset: None,
        } => out.write_str("［＃字下げ］"),
        LineFormat::Indent {
            amount,
            end_offset: None,
        } => write!(out, "［＃{amount}字下げ］"),
        LineFormat::AlignEnd { offset: 0 } => out.write_str("［＃地付き］"),
        LineFormat::AlignEnd { offset } => write!(out, "［＃地から{offset}字上げ］"),
        LineFormat::Center { page: true } => out.write_str("［＃ページの左右中央］"),
        LineFormat::Center { page: false } => out.write_str("［＃中央揃え］"),
        LineFormat::Framed(_) => out.write_str("［＃罫囲み］"),
        LineFormat::Gothic => out.write_str("［＃この行はゴシック体］"),
        // Absolute font-size line directive. `bold` canonicalises to `、太字`
        // (the classifier only admits that spelling, so the round-trip is exact).
        LineFormat::FontSizeAbsolute { size, bold } => {
            write!(
                out,
                "［＃{}{}］",
                size.keyword(),
                if bold { "、太字" } else { "" }
            )
        }
        // `LineFormat` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
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

/// `左に` left-side prefix for a bouten range marker, or `""`.
const fn bouten_left_prefix(position: BoutenPosition) -> &'static str {
    match position {
        BoutenPosition::Left => "左に",
        BoutenPosition::Both => "両側に",
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
        RegionFormat::Gothic { padded: false } => out.write_str("［＃ゴシック体］"),
        RegionFormat::Gothic { padded: true } => out.write_str("［＃ここからゴシック体］"),
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
        RegionFormat::Framed(_) => out.write_str("［＃罫囲み］"),
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
        gothic,
        horizontal,
        framed,
        font,
    } = styles;

    // `［＃ここからページの左右中央］` — a pure page-centred block (`amount: 0`,
    // `center`, no other clause). The short opener is its canonical spelling, so
    // emit it verbatim rather than the synthetic `ここから0字下げ、…` numbered
    // form, keeping `parse ∘ serialize` a fixed point for the source directive.
    if amount == 0
        && center
        && wrap.is_none()
        && matches!(layout, IndentLayout::None)
        && !gothic
        && !horizontal
        && !framed
        && font.is_none()
    {
        return out.write_str("［＃ここからページの左右中央］");
    }

    // The idiomatic no-number `［＃ここから字下げ］` form is reserved for a bare
    // single-char indent with no clauses; anything else takes the numbered form.
    let bare = wrap.is_none()
        && !center
        && matches!(layout, IndentLayout::None)
        && !gothic
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
    if gothic {
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
        RegionClose::Gothic { padded: false } => out.write_str("［＃ゴシック体終わり］"),
        RegionClose::Gothic { padded: true } => out.write_str("［＃ここでゴシック体終わり］"),
        RegionClose::Italic { padded: false } => out.write_str("［＃斜体終わり］"),
        RegionClose::Italic { padded: true } => out.write_str("［＃ここで斜体終わり］"),
        // #78 字組み compound — the close keeps its own width so the marker
        // round-trips byte-exact; every other indent close is the generic form.
        RegionClose::Indent {
            kumi_width: Some(width),
        } => write!(out, "［＃ここで字下げ、{}字組み終わり］", width.0),
        RegionClose::LineWidth => out.write_str("［＃ここで字詰め終わり］"),
        // Level-less bare close (`ここで見出し終わり` / `見出し終わり`): the open
        // payload drives pairing/render, so the close carries no level word.
        RegionClose::Heading {
            level: None,
            padded,
            ..
        } => write!(
            out,
            "［＃{}見出し終わり］",
            if padded { "ここで" } else { "" }
        ),
        RegionClose::Heading {
            level: Some(level),
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
        RegionClose::Framed(_) => out.write_str("［＃罫囲み終わり］"),
        RegionClose::AlignEnd => out.write_str("［＃ここで地付き終わり］"),
        // The generic `字下げ終わり` — the `Indent { kumi_width: None }` close
        // (plain / 字詰め / 折り返して / 中央 indents) and the `#[non_exhaustive]`
        // forward-compat fallback.
        _ => out.write_str("［＃ここで字下げ終わり］"),
    }
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
