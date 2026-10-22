#![expect(clippy::expect_used, reason = "fmt::Write into String is infallible")]

//! Shares source marker spellings between serialization and edits.

use core::fmt::{self, Write};

use crate::syntax::{
    BlockStyles, BoutenPosition, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, LineFormat,
    RegionClose, RegionFormat, SectionKind,
};

#[must_use]
#[cfg(test)]
pub(crate) fn container_open_source(open: RegionFormat) -> String {
    let mut s = String::new();
    emit_container_open(open, &mut s).expect("String write is infallible");
    s
}

#[must_use]
pub(crate) fn container_close_source(open: RegionFormat) -> String {
    let mut s = String::new();
    emit_container_close(RegionClose::of(open), &mut s).expect("String write is infallible");
    s
}

/// Tracks the last emitted character for ruby delimiter decisions.
pub(crate) struct TrackingWriter<W: Write> {
    inner: W,
    last: Option<char>,
}

impl<W: Write> TrackingWriter<W> {
    pub(crate) const fn new(inner: W) -> Self {
        Self { inner, last: None }
    }

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
        LineFormat::Gothic => out.write_str("［＃この行はゴシック体］"),
        LineFormat::FontSizeAbsolute { size, bold } => {
            write!(
                out,
                "［＃{}{}］",
                size.keyword(),
                if bold { "、太字" } else { "" }
            )
        }
    }
}

pub(crate) const fn heading_style_keyword(style: HeadingStyle) -> &'static str {
    match style {
        HeadingStyle::Standard => "",
        HeadingStyle::SameLine => "同行",
        HeadingStyle::Window => "窓",
    }
}

pub(crate) const fn heading_level_word(kind: HeadingKind) -> &'static str {
    match kind {
        HeadingKind::Large => "大見出し",
        HeadingKind::Medium => "中見出し",
        HeadingKind::Small => "小見出し",
    }
}

const fn bouten_left_prefix(position: BoutenPosition) -> &'static str {
    match position {
        BoutenPosition::Right => "",
        BoutenPosition::Left => "左に",
        BoutenPosition::Both => "両側に",
    }
}

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
        RegionFormat::Warichu => out.write_str("［＃ここから割り注］"),
        RegionFormat::Framed(_) => out.write_str("［＃罫囲み］"),
    }
}

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
        // The corpus uses `小さい活字` for a single reduction step.
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

const fn small_script_side_word(side: BoutenPosition) -> &'static str {
    match side {
        BoutenPosition::Right | BoutenPosition::Both => "右",
        BoutenPosition::Left => "左",
    }
}

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
        RegionClose::Indent {
            kumi_width: Some(width),
        } => write!(out, "［＃ここで字下げ、{}字組み終わり］", width.0),
        RegionClose::LineWidth => out.write_str("［＃ここで字詰め終わり］"),
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
        RegionClose::Indent { kumi_width: None } => out.write_str("［＃ここで字下げ終わり］"),
    }
}

/// Caps consecutive newlines to keep serialization stable across repeated parses.
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
    use crate::syntax::{
        AbsoluteSize, BoutenKind, ColumnCount, EnclosureKind, FontShift, Kumi, LineWidth,
    };
    use core::num::{NonZeroI8, NonZeroU8};
    use pretty_assertions::assert_eq;

    fn fs(n: i8) -> FontShift {
        FontShift(NonZeroI8::new(n).expect("nonzero shift"))
    }

    fn lw(n: u8) -> LineWidth {
        LineWidth(NonZeroU8::new(n).expect("nonzero width"))
    }

    fn cc(n: u8) -> ColumnCount {
        ColumnCount(NonZeroU8::new(n).expect("nonzero count"))
    }

    fn kumi(lines: u8, width: u8) -> Kumi {
        Kumi {
            lines: NonZeroU8::new(lines).expect("nonzero lines"),
            width: NonZeroU8::new(width).expect("nonzero width"),
        }
    }

    fn close_src(close: RegionClose) -> String {
        let mut s = String::new();
        emit_container_close(close, &mut s).expect("String write is infallible");
        s
    }

    fn line_src(lf: LineFormat) -> String {
        let mut s = String::new();
        emit_line(lf, &mut s).expect("String write is infallible");
        s
    }

    fn section_src(kind: SectionKind) -> String {
        let mut s = String::new();
        emit_section_break(kind, &mut s).expect("String write is infallible");
        s
    }

    #[test]
    fn public_marker_spellers_return_exact_bytes() {
        assert_eq!(
            container_open_source(RegionFormat::Bold { padded: false }),
            "［＃太字］"
        );
        assert_eq!(
            container_open_source(RegionFormat::Bold { padded: true }),
            "［＃ここから太字］"
        );
        assert_eq!(
            container_close_source(RegionFormat::Bold { padded: false }),
            "［＃太字終わり］"
        );
        assert_eq!(
            container_close_source(RegionFormat::Italic { padded: true }),
            "［＃ここで斜体終わり］"
        );
    }

    #[test]
    fn section_break_spells_each_kind() {
        assert_eq!(section_src(SectionKind::Kaicho), "［＃改丁］");
        assert_eq!(section_src(SectionKind::Kaidan), "［＃改段］");
        assert_eq!(section_src(SectionKind::Kaimihiraki), "［＃改見開き］");
    }

    #[test]
    fn line_format_spells_every_arm() {
        assert_eq!(
            line_src(LineFormat::Indent {
                amount: 2,
                end_offset: Some(3),
            }),
            "［＃2字下げ、地より3字上げで］"
        );
        assert_eq!(
            line_src(LineFormat::Indent {
                amount: 1,
                end_offset: None,
            }),
            "［＃字下げ］"
        );
        assert_eq!(
            line_src(LineFormat::Indent {
                amount: 3,
                end_offset: None,
            }),
            "［＃3字下げ］"
        );
        assert_eq!(line_src(LineFormat::AlignEnd { offset: 0 }), "［＃地付き］");
        assert_eq!(
            line_src(LineFormat::AlignEnd { offset: 4 }),
            "［＃地から4字上げ］"
        );
        assert_eq!(
            line_src(LineFormat::Center { page: true }),
            "［＃ページの左右中央］"
        );
        assert_eq!(
            line_src(LineFormat::Center { page: false }),
            "［＃中央揃え］"
        );
        assert_eq!(line_src(LineFormat::Gothic), "［＃この行はゴシック体］");
        assert_eq!(
            line_src(LineFormat::FontSizeAbsolute {
                size: AbsoluteSize::Large,
                bold: false,
            }),
            "［＃大文字］"
        );
        assert_eq!(
            line_src(LineFormat::FontSizeAbsolute {
                size: AbsoluteSize::ExtraLarge,
                bold: true,
            }),
            "［＃特大文字、太字］"
        );
    }

    #[test]
    fn heading_style_keyword_prefixes() {
        assert_eq!(heading_style_keyword(HeadingStyle::Standard), "");
        assert_eq!(heading_style_keyword(HeadingStyle::SameLine), "同行");
        assert_eq!(heading_style_keyword(HeadingStyle::Window), "窓");
    }

    #[test]
    fn heading_level_word_per_level() {
        assert_eq!(heading_level_word(HeadingKind::Large), "大見出し");
        assert_eq!(heading_level_word(HeadingKind::Medium), "中見出し");
        assert_eq!(heading_level_word(HeadingKind::Small), "小見出し");
    }

    #[test]
    fn bouten_left_prefix_per_position() {
        assert_eq!(bouten_left_prefix(BoutenPosition::Right), "");
        assert_eq!(bouten_left_prefix(BoutenPosition::Left), "左に");
        assert_eq!(bouten_left_prefix(BoutenPosition::Both), "両側に");
    }

    #[test]
    fn small_script_side_word_per_side() {
        assert_eq!(small_script_side_word(BoutenPosition::Right), "右");
        assert_eq!(small_script_side_word(BoutenPosition::Left), "左");
    }

    #[test]
    fn container_open_spells_inline_styles() {
        let open = container_open_source;
        assert_eq!(
            open(RegionFormat::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }),
            "［＃傍点］"
        );
        assert_eq!(
            open(RegionFormat::Bouten {
                kind: BoutenKind::UnderLine,
                position: BoutenPosition::Left,
            }),
            "［＃左に傍線］"
        );
        assert_eq!(open(RegionFormat::Bold { padded: false }), "［＃太字］");
        assert_eq!(
            open(RegionFormat::Bold { padded: true }),
            "［＃ここから太字］"
        );
        assert_eq!(
            open(RegionFormat::Gothic { padded: false }),
            "［＃ゴシック体］"
        );
        assert_eq!(
            open(RegionFormat::Gothic { padded: true }),
            "［＃ここからゴシック体］"
        );
        assert_eq!(open(RegionFormat::Italic { padded: false }), "［＃斜体］");
        assert_eq!(
            open(RegionFormat::Italic { padded: true }),
            "［＃ここから斜体］"
        );
    }

    #[test]
    fn container_open_spells_block_layouts() {
        let open = container_open_source;
        assert_eq!(
            open(RegionFormat::AlignEnd { offset: 0 }),
            "［＃ここから地付き］"
        );
        assert_eq!(
            open(RegionFormat::AlignEnd { offset: 2 }),
            "［＃ここから地から2字上げ］"
        );
        assert_eq!(
            open(RegionFormat::LineWidth(lw(20))),
            "［＃ここから20字詰め］"
        );
        assert_eq!(
            open(RegionFormat::Heading {
                level: HeadingKind::Large,
                style: HeadingStyle::Standard,
                padded: true,
            }),
            "［＃ここから大見出し］"
        );
        assert_eq!(
            open(RegionFormat::Heading {
                level: HeadingKind::Medium,
                style: HeadingStyle::SameLine,
                padded: false,
            }),
            "［＃同行中見出し］"
        );
        assert_eq!(open(RegionFormat::Columns(cc(2))), "［＃ここから2段組み］");
        assert_eq!(open(RegionFormat::Table), "［＃ここから表］");
        assert_eq!(open(RegionFormat::Horizontal), "［＃ここから横組み］");
        assert_eq!(
            open(RegionFormat::FontSize(fs(2))),
            "［＃ここから2段階大きな文字］"
        );
        assert_eq!(
            open(RegionFormat::FontSize(fs(-3))),
            "［＃ここから3段階小さな文字］"
        );
        assert_eq!(
            open(RegionFormat::SmallScript(BoutenPosition::Right)),
            "［＃行右小書き］"
        );
        assert_eq!(
            open(RegionFormat::SmallScript(BoutenPosition::Left)),
            "［＃行左小書き］"
        );
        assert_eq!(
            open(RegionFormat::Caption { padded: true }),
            "［＃ここからキャプション］"
        );
        assert_eq!(
            open(RegionFormat::Caption { padded: false }),
            "［＃キャプション］"
        );
        assert_eq!(open(RegionFormat::Warichu), "［＃ここから割り注］");
        assert_eq!(
            open(RegionFormat::Framed(EnclosureKind::Rule)),
            "［＃罫囲み］"
        );
    }

    #[test]
    fn indent_open_page_centre_short_form() {
        assert_eq!(
            container_open_source(RegionFormat::Indent(IndentBlock {
                amount: 0,
                wrap: None,
                center: true,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            })),
            "［＃ここからページの左右中央］"
        );
    }

    #[test]
    fn indent_open_page_centre_guard_needs_every_clause() {
        let base = IndentBlock {
            amount: 0,
            wrap: None,
            center: true,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        };
        let cases: &[(IndentBlock, &str)] = &[
            (
                IndentBlock {
                    center: false,
                    ..base
                },
                "［＃ここから0字下げ］",
            ),
            (
                IndentBlock {
                    wrap: Some(2),
                    ..base
                },
                "［＃ここから0字下げ、折り返して2字下げ、ページの左右中央に］",
            ),
            (
                IndentBlock {
                    layout: IndentLayout::LineWidth(lw(3)),
                    ..base
                },
                "［＃ここから0字下げ、ページの左右中央に、3字詰め］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        gothic: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから0字下げ、ページの左右中央に、ゴシック体］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        horizontal: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから0字下げ、ページの左右中央に、横書き］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        framed: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから0字下げ、ページの左右中央に、罫囲み］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        font: Some(fs(2)),
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから0字下げ、ページの左右中央に、2段階大きな文字］",
            ),
        ];
        for (block, expected) in cases {
            assert_eq!(
                container_open_source(RegionFormat::Indent(*block)),
                *expected,
                "page-centre guard row: {block:?}",
            );
        }
    }

    #[test]
    fn indent_open_bare_single_char_short_form() {
        assert_eq!(
            container_open_source(RegionFormat::Indent(IndentBlock {
                amount: 1,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            })),
            "［＃ここから字下げ］"
        );
    }

    #[test]
    fn indent_open_bare_guard_needs_every_clause() {
        let base = IndentBlock {
            amount: 1,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        };
        let cases: &[(IndentBlock, &str)] = &[
            (
                IndentBlock {
                    center: true,
                    ..base
                },
                "［＃ここから1字下げ、ページの左右中央に］",
            ),
            (
                IndentBlock {
                    layout: IndentLayout::LineWidth(lw(3)),
                    ..base
                },
                "［＃ここから1字下げ、3字詰め］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        gothic: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから1字下げ、ゴシック体］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        horizontal: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから1字下げ、横書き］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        framed: true,
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから1字下げ、罫囲み］",
            ),
            (
                IndentBlock {
                    styles: BlockStyles {
                        font: Some(fs(-2)),
                        ..BlockStyles::EMPTY
                    },
                    ..base
                },
                "［＃ここから1字下げ、2段階小さな文字］",
            ),
        ];
        for (block, expected) in cases {
            assert_eq!(
                container_open_source(RegionFormat::Indent(*block)),
                *expected,
                "bare guard row: {block:?}",
            );
        }
    }

    #[test]
    fn indent_open_numbered_compound_clauses() {
        assert_eq!(
            container_open_source(RegionFormat::Indent(IndentBlock {
                amount: 2,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles {
                    font: Some(fs(-1)),
                    ..BlockStyles::EMPTY
                },
            })),
            "［＃ここから2字下げ、小さい活字］"
        );
        assert_eq!(
            container_open_source(RegionFormat::Indent(IndentBlock {
                amount: 2,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles {
                    font: Some(fs(-2)),
                    ..BlockStyles::EMPTY
                },
            })),
            "［＃ここから2字下げ、2段階小さな文字］"
        );
        assert_eq!(
            container_open_source(RegionFormat::Indent(IndentBlock {
                amount: 2,
                wrap: None,
                center: false,
                layout: IndentLayout::Kumi(kumi(5, 3)),
                styles: BlockStyles::EMPTY,
            })),
            "［＃ここから2字下げ、5行3字組みで］"
        );
    }

    #[test]
    fn container_close_spells_inline_styles() {
        assert_eq!(
            close_src(RegionClose::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }),
            "［＃傍点終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Bouten {
                kind: BoutenKind::UnderLine,
                position: BoutenPosition::Left,
            }),
            "［＃左に傍線終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Bold { padded: false }),
            "［＃太字終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Bold { padded: true }),
            "［＃ここで太字終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Gothic { padded: false }),
            "［＃ゴシック体終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Gothic { padded: true }),
            "［＃ここでゴシック体終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Italic { padded: false }),
            "［＃斜体終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Italic { padded: true }),
            "［＃ここで斜体終わり］"
        );
    }

    #[test]
    fn container_close_spells_indents_and_headings() {
        assert_eq!(
            close_src(RegionClose::Indent {
                kumi_width: Some(lw(5)),
            }),
            "［＃ここで字下げ、5字組み終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Indent { kumi_width: None }),
            "［＃ここで字下げ終わり］"
        );
        assert_eq!(
            close_src(RegionClose::LineWidth),
            "［＃ここで字詰め終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Heading {
                level: None,
                style: HeadingStyle::Standard,
                padded: true,
            }),
            "［＃ここで見出し終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Heading {
                level: None,
                style: HeadingStyle::Standard,
                padded: false,
            }),
            "［＃見出し終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Heading {
                level: Some(HeadingKind::Medium),
                style: HeadingStyle::Standard,
                padded: false,
            }),
            "［＃中見出し終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Heading {
                level: Some(HeadingKind::Small),
                style: HeadingStyle::Window,
                padded: true,
            }),
            "［＃ここで窓小見出し終わり］"
        );
    }

    #[test]
    fn container_close_spells_other_families() {
        assert_eq!(close_src(RegionClose::Columns), "［＃ここで段組み終わり］");
        assert_eq!(close_src(RegionClose::Table), "［＃ここで表終わり］");
        assert_eq!(
            close_src(RegionClose::Horizontal),
            "［＃ここで横組み終わり］"
        );
        assert_eq!(
            close_src(RegionClose::FontSize { larger: true }),
            "［＃ここで大きな文字終わり］"
        );
        assert_eq!(
            close_src(RegionClose::FontSize { larger: false }),
            "［＃ここで小さな文字終わり］"
        );
        assert_eq!(
            close_src(RegionClose::SmallScript(BoutenPosition::Right)),
            "［＃行右小書き終わり］"
        );
        assert_eq!(
            close_src(RegionClose::SmallScript(BoutenPosition::Left)),
            "［＃行左小書き終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Caption { padded: true }),
            "［＃ここでキャプション終わり］"
        );
        assert_eq!(
            close_src(RegionClose::Caption { padded: false }),
            "［＃キャプション終わり］"
        );
        assert_eq!(close_src(RegionClose::Warichu), "［＃ここで割り注終わり］");
        assert_eq!(
            close_src(RegionClose::Framed(EnclosureKind::Rule)),
            "［＃罫囲み終わり］"
        );
        assert_eq!(close_src(RegionClose::AlignEnd), "［＃ここで地付き終わり］");
    }

    #[test]
    fn newline_cap_persists_across_writes() {
        let mut w = NewlineCappedWriter::with_capacity(16);
        w.write_str("a\n\n").expect("infallible");
        w.write_str("\n\n").expect("infallible");
        assert_eq!(w.into_string(), "a\n\n");
    }

    #[test]
    fn newline_cap_within_one_write() {
        let mut w = NewlineCappedWriter::with_capacity(16);
        w.write_str("x\n\n\n\n\ny").expect("infallible");
        assert_eq!(w.into_string(), "x\n\ny");
    }

    #[test]
    fn tracking_writer_last_remembers_final_char() {
        let mut buf = String::new();
        let mut tw = TrackingWriter::new(&mut buf);
        assert_eq!(tw.last(), None);
        tw.write_str("あい").expect("String write is infallible");
        assert_eq!(
            tw.last(),
            Some('い'),
            "last() must return the final char written"
        );
    }

    #[test]
    fn write_char_copies_plain_char() {
        let mut w = NewlineCappedWriter::with_capacity(4);
        w.write_char('x').expect("infallible");
        assert_eq!(w.into_string(), "x");
    }

    #[test]
    fn write_char_caps_newline_run() {
        let mut w = NewlineCappedWriter::with_capacity(8);
        for _ in 0..4 {
            w.write_char('\n').expect("infallible");
        }
        assert_eq!(w.into_string(), "\n\n");
    }

    #[test]
    fn write_char_emits_first_newline() {
        let mut w = NewlineCappedWriter::with_capacity(4);
        w.write_char('\n').expect("infallible");
        w.write_char('x').expect("infallible");
        assert_eq!(w.into_string(), "\nx");
    }
}
