//! Defines notation kinds and the parser's owned syntax tree.

#![forbid(unsafe_code)]

const SMALL_HEADING_OUTLINE_LEVEL: u8 = 3;

pub(crate) mod accent;
pub(crate) mod alloc;
pub(crate) mod ast;
pub(crate) mod degraded;
pub(crate) mod format;
pub(crate) mod lint;
pub(crate) mod node_kind;

pub(crate) use format::{
    AbsoluteSize, AccentMark, BlockStyles, ColumnCount, EnclosureKind, FontShift, ForwardAttr,
    ForwardOrigin, IndentBlock, IndentLayout, Kumi, LineFormat, LineWidth, RegionClose,
    RegionFormat,
};
pub use node_kind::NodeKind;

pub(crate) use crate::encoding::gaiji::GaijiCanonical;

pub(crate) use crate::spec::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct Container {
    pub kind: RegionFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    Goma,
    WhiteSesame,
    Circle,
    WhiteCircle,
    DoubleCircle,
    Janome,
    Cross,
    WhiteTriangle,
    WavyLine,
    UnderLine,
    DoubleUnderLine,
    ChainLine,
    DashedLine,
    BlackTriangle,
}

impl BoutenKind {
    #[must_use]
    pub const fn is_line(self) -> bool {
        matches!(
            self,
            Self::WavyLine
                | Self::UnderLine
                | Self::DoubleUnderLine
                | Self::ChainLine
                | Self::DashedLine
        )
    }

    #[must_use]
    pub const fn family_str(self) -> &'static str {
        if self.is_line() { "傍線" } else { "傍点" }
    }
}

pub(crate) const BOUTEN_KINDS: &[BoutenKind] = &[
    BoutenKind::Goma,
    BoutenKind::WhiteSesame,
    BoutenKind::Circle,
    BoutenKind::WhiteCircle,
    BoutenKind::DoubleCircle,
    BoutenKind::Janome,
    BoutenKind::Cross,
    BoutenKind::WhiteTriangle,
    BoutenKind::WavyLine,
    BoutenKind::UnderLine,
    BoutenKind::DoubleUnderLine,
    BoutenKind::ChainLine,
    BoutenKind::DashedLine,
    BoutenKind::BlackTriangle,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    #[default]
    Right,
    Left,
    Both,
}

/// Placement of a ruby reading relative to its base text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RubySide {
    /// Right of the base in vertical text, or over it in horizontal text.
    #[default]
    Right,
    /// Left of the base in vertical text, or below it in horizontal text.
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub(crate) enum RubyBaseClass {
    Kanji,
    Hiragana,
    Katakana,
    Latin,
    Greek,
    Symbol,
}

#[must_use]
pub(crate) const fn ruby_base_class(ch: char) -> Option<RubyBaseClass> {
    Some(match ch {
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆' => RubyBaseClass::Kanji,
        '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' => RubyBaseClass::Hiragana,
        '\u{30A1}'..='\u{30F4}' | '\u{30F7}'..='\u{30FA}' | '\u{30FC}'..='\u{30FE}' => {
            RubyBaseClass::Katakana
        }
        'A'..='Z'
        | 'a'..='z'
        | 'Ａ'..='Ｚ'
        | 'ａ'..='ｚ'
        | '\u{00C0}'..='\u{00D6}'
        | '\u{00D8}'..='\u{00F6}'
        | '\u{00F8}'..='\u{02AF}'
        | '\u{1E00}'..='\u{1EFF}'
        | '\u{A720}'..='\u{A7FF}'
        | '\u{AB30}'..='\u{AB6F}'
        | '\u{FB00}'..='\u{FB06}' => RubyBaseClass::Latin,
        '\u{0391}'..='\u{03A9}' | '\u{03B1}'..='\u{03C9}' => RubyBaseClass::Greek,
        '+' | '＋' => RubyBaseClass::Symbol,
        _ => return None,
    })
}

#[must_use]
#[cfg(test)]
pub(crate) const fn is_ruby_base_char(ch: char) -> bool {
    matches!(ruby_base_class(ch), Some(RubyBaseClass::Kanji))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum MarginNoteKind {
    #[default]
    Gloss,
    Marginal,
}

impl MarginNoteKind {
    #[must_use]
    pub(crate) const fn serialize_affixes(self) -> (&'static str, &'static str) {
        match self {
            Self::Gloss => ("」の左に「", "」の注記］"),
            Self::Marginal => ("」に「", "」の傍記］"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum SectionKind {
    Kaicho,
    Kaidan,
    Kaimihiraki,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingKind {
    Large,
    Medium,
    Small,
}

impl HeadingKind {
    #[must_use]
    pub const fn outline_level(self) -> u8 {
        match self {
            Self::Medium => 2,
            Self::Small => SMALL_HEADING_OUTLINE_LEVEL,
            _ => 1,
        }
    }
}

/// Corpus sources contain no `副見出し` annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingStyle {
    #[default]
    Standard,
    SameLine,
    Window,
}

#[cfg(test)]
pub(crate) const SECTION_KINDS: &[SectionKind] = &[
    SectionKind::Kaicho,
    SectionKind::Kaidan,
    SectionKind::Kaimihiraki,
];

#[cfg(test)]
pub(crate) const HEADING_KINDS: &[HeadingKind] =
    &[HeadingKind::Large, HeadingKind::Medium, HeadingKind::Small];

#[cfg(test)]
pub(crate) const HEADING_STYLES: &[HeadingStyle] = &[
    HeadingStyle::Standard,
    HeadingStyle::SameLine,
    HeadingStyle::Window,
];

impl BoutenKind {
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::WhiteSesame => "白ゴマ傍点",
            Self::Circle => "丸傍点",
            Self::WhiteCircle => "白丸傍点",
            Self::DoubleCircle => "二重丸傍点",
            Self::Janome => "蛇の目傍点",
            Self::Cross => "ばつ傍点",
            Self::WhiteTriangle => "白三角傍点",
            Self::WavyLine => "波線",
            Self::UnderLine => "傍線",
            Self::DoubleUnderLine => "二重傍線",
            Self::ChainLine => "鎖線",
            Self::DashedLine => "破線",
            Self::BlackTriangle => "黒三角傍点",
            _ => "傍点",
        }
    }
}

impl SectionKind {
    #[must_use]
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Kaicho => "改丁",
            Self::Kaidan => "改段",
            Self::Kaimihiraki => "改見開き",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DirectiveKind {
    NonCanonical,
    Editorial,
    Sic,
    BaseTextVariant,
    WarichuOpen,
    WarichuClose,
    Empty,
    EditorNote,
    RubyAttached,
    RubyRetarget,
    RubyPairOpen,
    RubyPairClose,
    MarginNotePairOpen,
    MarginNotePairClose,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::roman_slug;

    #[test]
    fn empty_span_is_empty_and_zero_length() {
        let s = Span::new(42, 42);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn span_slices_source_buffer() {
        let source = "hello world";
        let s = Span::new(6, 11);
        assert_eq!(s.slice(source), "world");
    }

    #[test]
    fn bouten_position_defaults_to_right() {
        assert_eq!(BoutenPosition::default(), BoutenPosition::Right);
    }

    #[test]
    fn ruby_side_and_heading_style_defaults() {
        assert_eq!(RubySide::default(), RubySide::Right);
        assert_eq!(HeadingStyle::default(), HeadingStyle::Standard);
    }

    #[test]
    fn ruby_base_class_maps_each_class_and_its_range_boundaries() {
        use RubyBaseClass::{Greek, Hiragana, Kanji, Katakana, Latin, Symbol};
        let cases: &[(char, RubyBaseClass)] = &[
            ('\u{3400}', Kanji),
            ('\u{4DBF}', Kanji),
            ('\u{4E00}', Kanji),
            ('\u{9FFF}', Kanji),
            ('\u{F900}', Kanji),
            ('\u{FAFF}', Kanji),
            ('\u{20000}', Kanji),
            ('\u{2FFFF}', Kanji),
            ('々', Kanji),
            ('〆', Kanji),
            ('漢', Kanji),
            ('\u{3041}', Hiragana),
            ('\u{3096}', Hiragana),
            ('\u{309D}', Hiragana),
            ('\u{309E}', Hiragana),
            ('あ', Hiragana),
            ('\u{30A1}', Katakana),
            ('\u{30F4}', Katakana),
            ('\u{30F7}', Katakana),
            ('\u{30FA}', Katakana),
            ('\u{30FC}', Katakana),
            ('\u{30FE}', Katakana),
            ('カ', Katakana),
            ('A', Latin),
            ('Z', Latin),
            ('a', Latin),
            ('z', Latin),
            ('Ａ', Latin),
            ('Ｚ', Latin),
            ('ａ', Latin),
            ('ｚ', Latin),
            ('é', Latin),
            ('É', Latin),
            ('Œ', Latin),
            ('\u{0391}', Greek),
            ('\u{03A9}', Greek),
            ('\u{03B1}', Greek),
            ('\u{03C9}', Greek),
            ('+', Symbol),
            ('＋', Symbol),
        ];
        for &(ch, class) in cases {
            assert_eq!(
                ruby_base_class(ch),
                Some(class),
                "U+{:04X} should classify as {class:?}",
                ch as u32
            );
        }
        let none_cases: &[char] = &[
            'ヵ', 'ヶ', '・', '\u{33FF}', '\u{3040}', '\u{3097}', '\u{30FF}', '\u{0390}',
            '\u{03AA}', '\u{03CA}', ' ', '5', '。',
        ];
        for &ch in none_cases {
            assert_eq!(
                ruby_base_class(ch),
                None,
                "U+{:04X} must not classify as any base class",
                ch as u32
            );
        }
    }

    #[test]
    fn is_ruby_base_char_is_exactly_the_kanji_class() {
        assert!(is_ruby_base_char('漢'));
        assert!(is_ruby_base_char('々'));
        assert!(is_ruby_base_char('\u{4E00}'));
        assert!(!is_ruby_base_char('あ'));
        assert!(!is_ruby_base_char('カ'));
        assert!(!is_ruby_base_char('A'));
        assert!(!is_ruby_base_char('\u{0391}'));
        assert!(!is_ruby_base_char(' '));
    }

    #[test]
    fn margin_note_kind_serialize_affixes_are_exact() {
        assert_eq!(
            MarginNoteKind::Gloss.serialize_affixes(),
            ("」の左に「", "」の注記］")
        );
        assert_eq!(
            MarginNoteKind::Marginal.serialize_affixes(),
            ("」に「", "」の傍記］")
        );
    }

    #[test]
    fn heading_kind_outline_level_maps_each_rank() {
        assert_eq!(HeadingKind::Large.outline_level(), 1);
        assert_eq!(HeadingKind::Medium.outline_level(), 2);
        assert_eq!(HeadingKind::Small.outline_level(), 3);
    }

    #[test]
    fn bouten_keyword_is_exhaustive_and_stable() {
        let cases = [
            (BoutenKind::Goma, "傍点"),
            (BoutenKind::WhiteSesame, "白ゴマ傍点"),
            (BoutenKind::Circle, "丸傍点"),
            (BoutenKind::WhiteCircle, "白丸傍点"),
            (BoutenKind::DoubleCircle, "二重丸傍点"),
            (BoutenKind::Janome, "蛇の目傍点"),
            (BoutenKind::Cross, "ばつ傍点"),
            (BoutenKind::WhiteTriangle, "白三角傍点"),
            (BoutenKind::WavyLine, "波線"),
            (BoutenKind::UnderLine, "傍線"),
            (BoutenKind::DoubleUnderLine, "二重傍線"),
            (BoutenKind::ChainLine, "鎖線"),
            (BoutenKind::DashedLine, "破線"),
            (BoutenKind::BlackTriangle, "黒三角傍点"),
        ];
        for (kind, kw) in cases {
            assert_eq!(kind.keyword(), kw, "keyword mismatch for {kind:?}");
        }
    }

    #[test]
    fn bouten_is_line_splits_dot_from_line_family() {
        for kind in [
            BoutenKind::WavyLine,
            BoutenKind::UnderLine,
            BoutenKind::DoubleUnderLine,
            BoutenKind::ChainLine,
            BoutenKind::DashedLine,
        ] {
            assert!(kind.is_line(), "{kind:?} should be a 線 variant");
            assert_eq!(kind.family_str(), "傍線", "family_str for {kind:?}");
        }
        for kind in [
            BoutenKind::Goma,
            BoutenKind::WhiteSesame,
            BoutenKind::Circle,
            BoutenKind::WhiteCircle,
            BoutenKind::DoubleCircle,
            BoutenKind::Janome,
            BoutenKind::Cross,
            BoutenKind::WhiteTriangle,
            BoutenKind::BlackTriangle,
        ] {
            assert!(!kind.is_line(), "{kind:?} should be a 点 variant");
            assert_eq!(kind.family_str(), "傍点", "family_str for {kind:?}");
        }
    }

    #[test]
    fn forward_attr_keyword_is_exhaustive_and_stable() {
        use core::num::NonZeroI8;
        let cases = [
            (ForwardAttr::Bold, "太字"),
            (ForwardAttr::Gothic, "ゴシック体"),
            (ForwardAttr::Italic, "斜体"),
            (ForwardAttr::SuperScript, "上付き小文字"),
            (ForwardAttr::SubScript, "下付き小文字"),
            (
                ForwardAttr::SmallScript(BoutenPosition::Right),
                "行右小書き",
            ),
            (ForwardAttr::SmallScript(BoutenPosition::Left), "行左小書き"),
            (ForwardAttr::Framed(EnclosureKind::Rule), "罫囲み"),
            (ForwardAttr::Framed(EnclosureKind::Box), "囲み"),
            (ForwardAttr::Framed(EnclosureKind::Circle), "○付き文字"),
            (
                ForwardAttr::Framed(EnclosureKind::CircleDotted),
                "点線丸囲み",
            ),
            (ForwardAttr::Framed(EnclosureKind::DoubleRule), "二重罫囲み"),
            (ForwardAttr::Horizontal, "横組み"),
            (ForwardAttr::Caption, "キャプション"),
            (ForwardAttr::CombineUpright, "縦中横"),
            (ForwardAttr::Fraction, "分数"),
            (
                ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
                "特大文字",
            ),
            (ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large), "大文字"),
            (
                ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
                "中文字",
            ),
            (ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small), "小文字"),
            (
                ForwardAttr::Bouten {
                    kind: BoutenKind::Goma,
                    position: BoutenPosition::Right,
                },
                "傍点",
            ),
            (
                ForwardAttr::FontSize(FontShift(NonZeroI8::new(3).unwrap())),
                "太字",
            ),
            (ForwardAttr::AccentDot, "太字"),
            (ForwardAttr::Accent(AccentMark::Acute), "太字"),
        ];
        for (attr, kw) in cases {
            assert_eq!(attr.keyword(), kw, "keyword mismatch for {attr:?}");
        }
    }

    #[test]
    fn section_keyword_is_exhaustive() {
        assert_eq!(SectionKind::Kaicho.keyword(), "改丁");
        assert_eq!(SectionKind::Kaidan.keyword(), "改段");
        assert_eq!(SectionKind::Kaimihiraki.keyword(), "改見開き");
    }

    #[test]
    fn heading_kind_and_style_are_orthogonal_copies() {
        let heading = |level, style| Container {
            kind: RegionFormat::Heading {
                level,
                style,
                padded: true,
            },
        };
        assert_eq!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Medium, HeadingStyle::Window),
            "equal Heading containers compare equal"
        );
        assert_ne!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Small, HeadingStyle::Window),
            "different level ⇒ not equal"
        );
        assert_eq!(
            LineFormat::Indent {
                amount: 4,
                end_offset: None
            },
            LineFormat::Indent {
                amount: 4,
                end_offset: None
            }
        );
        assert!(matches!(
            LineFormat::AlignEnd { offset: 0 },
            LineFormat::AlignEnd { offset: 0 }
        ));
        assert!(matches!(
            LineFormat::Center { page: true },
            LineFormat::Center { page: true }
        ));
    }

    #[test]
    fn bouten_kinds_are_complete_and_distinct() {
        assert_eq!(BOUTEN_KINDS.len(), 14, "every BoutenKind variant listed");
        for (i, a) in BOUTEN_KINDS.iter().enumerate() {
            for b in &BOUTEN_KINDS[i + 1..] {
                assert_ne!(a, b, "duplicate variant in BOUTEN_KINDS");
                assert_ne!(a.keyword(), b.keyword(), "duplicate bouten keyword");
            }
        }
    }

    #[test]
    fn every_bouten_kind_has_a_render_slug() {
        for k in BOUTEN_KINDS {
            assert!(
                roman_slug(k.keyword()).is_some(),
                "RENDER_SLUGS missing a slug for bouten keyword {:?}",
                k.keyword()
            );
        }
    }
}
