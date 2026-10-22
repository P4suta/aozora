use core::num::{NonZeroI8, NonZeroU8};

use crate::syntax::{BoutenKind, BoutenPosition, HeadingKind, HeadingStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct FontShift(pub NonZeroI8);

impl FontShift {
    #[must_use]
    pub const fn larger(self) -> bool {
        self.0.get().is_positive()
    }

    #[must_use]
    pub const fn magnitude(self) -> u8 {
        self.0.get().unsigned_abs()
    }
}

// The corpus work `暗黒公使` names absolute headline sizes rather than relative shifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AbsoluteSize {
    ExtraLarge,
    Large,
    Medium,
    Small,
}

impl AbsoluteSize {
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::ExtraLarge => "特大文字",
            Self::Large => "大文字",
            Self::Medium => "中文字",
            Self::Small => "小文字",
        }
    }
}

// ADR-0027 declines non-canonical enclosure spellings to preserve their source meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum EnclosureKind {
    Rule,
    Box,
    Circle,
    CircleDotted,
    DoubleRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub(crate) enum AccentMark {
    Acute,
    Umlaut,
    Grave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnCount(pub NonZeroU8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct LineWidth(pub NonZeroU8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct Kumi {
    pub lines: NonZeroU8,
    pub width: NonZeroU8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum IndentLayout {
    None,
    LineWidth(LineWidth),
    Kumi(Kumi),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockStyles {
    pub gothic: bool,
    pub horizontal: bool,
    pub framed: bool,
    pub font: Option<FontShift>,
}

impl BlockStyles {
    pub const EMPTY: Self = Self {
        gothic: false,
        horizontal: false,
        framed: false,
        font: None,
    };

    #[must_use]
    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }

    pub fn iter_formats(self) -> impl Iterator<Item = Format> {
        let Self {
            gothic,
            horizontal,
            framed,
            font,
        } = self;
        [
            gothic.then_some(Format::Gothic),
            horizontal.then_some(Format::Horizontal),
            framed.then_some(Format::Framed(EnclosureKind::Rule)),
            font.map(Format::FontSize),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct IndentBlock {
    pub amount: u8,
    pub wrap: Option<u8>,
    pub center: bool,
    pub layout: IndentLayout,
    pub styles: BlockStyles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Format {
    Bold,
    // ADR-0027 distinguishes gothic from bold based on their disjoint corpus use.
    Gothic,
    Italic,
    Bouten(BoutenKind),
    Framed(EnclosureKind),
    Horizontal,
    FontSize(FontShift),
    FontSizeAbsolute(AbsoluteSize),
    Caption,
    SuperScript,
    SubScript,
    SmallScript(BoutenPosition),
    CombineUpright,
    Fraction,
    AccentDot,
    Accent,
    Indent,
    AlignEnd,
    Center,
    LineWidth,
    Table,
    Columns(ColumnCount),
    Warichu,
    Heading {
        level: HeadingKind,
        style: HeadingStyle,
    },
}

impl Format {
    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Gothic => "gothic",
            Self::Italic => "italic",
            Self::Bouten(_) => "bouten",
            Self::Framed(_) => "framed",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "fontSize",
            Self::FontSizeAbsolute(_) => "fontSizeAbsolute",
            Self::Caption => "caption",
            Self::SuperScript => "superScript",
            Self::SubScript => "subScript",
            Self::SmallScript(_) => "smallScript",
            Self::CombineUpright => "combineUpright",
            Self::Fraction => "fraction",
            Self::AccentDot => "accentDot",
            Self::Accent => "accent",
            Self::Indent => "indent",
            Self::AlignEnd => "alignEnd",
            Self::Center => "center",
            Self::LineWidth => "lineWidth",
            Self::Table => "table",
            Self::Columns(_) => "columns",
            Self::Warichu => "warichu",
            Self::Heading { .. } => "heading",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum ForwardAttr {
    Bold,
    Gothic,
    Italic,
    SuperScript,
    SubScript,
    SmallScript(BoutenPosition),
    Framed(EnclosureKind),
    Horizontal,
    Caption,
    FontSize(FontShift),
    FontSizeAbsolute(AbsoluteSize),
    Bouten {
        kind: BoutenKind,
        position: BoutenPosition,
    },
    CombineUpright,
    Fraction,
    AccentDot,
    Accent(AccentMark),
    AlignEnd {
        offset: u8,
    },
}

// ADR-0019 requires provenance to distinguish ruby-base referents from reclaimable text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub(crate) enum ForwardOrigin {
    Reclaimed,
    Referenced,
    SelfContained,
    Detached,
}

impl ForwardOrigin {
    #[must_use]
    pub(crate) const fn from_consume(consume_start: u32, bracket_start: u32) -> Self {
        if consume_start < bracket_start {
            Self::Reclaimed
        } else {
            Self::Referenced
        }
    }
}

impl ForwardAttr {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn format(self) -> Format {
        match self {
            Self::Bold => Format::Bold,
            Self::Gothic => Format::Gothic,
            Self::Italic => Format::Italic,
            Self::SuperScript => Format::SuperScript,
            Self::SubScript => Format::SubScript,
            Self::SmallScript(p) => Format::SmallScript(p),
            Self::Framed(k) => Format::Framed(k),
            Self::Horizontal => Format::Horizontal,
            Self::Caption => Format::Caption,
            Self::FontSize(f) => Format::FontSize(f),
            Self::FontSizeAbsolute(s) => Format::FontSizeAbsolute(s),
            Self::Bouten { kind, .. } => Format::Bouten(kind),
            Self::CombineUpright => Format::CombineUpright,
            Self::Fraction => Format::Fraction,
            Self::AccentDot => Format::AccentDot,
            Self::Accent(_) => Format::Accent,
            Self::AlignEnd { .. } => Format::AlignEnd,
        }
    }

    #[must_use]
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Gothic => "ゴシック体",
            Self::Italic => "斜体",
            Self::SuperScript => "上付き小文字",
            Self::SubScript => "下付き小文字",
            Self::SmallScript(BoutenPosition::Right) => "行右小書き",
            Self::SmallScript(BoutenPosition::Left) => "行左小書き",
            Self::Framed(EnclosureKind::Rule) => "罫囲み",
            Self::Framed(EnclosureKind::Box) => "囲み",
            Self::Framed(EnclosureKind::Circle) => "○付き文字",
            Self::Framed(EnclosureKind::CircleDotted) => "点線丸囲み",
            Self::Framed(EnclosureKind::DoubleRule) => "二重罫囲み",
            Self::Horizontal => "横組み",
            Self::Caption => "キャプション",
            Self::CombineUpright => "縦中横",
            Self::Fraction => "分数",
            Self::FontSizeAbsolute(s) => s.keyword(),
            Self::Bouten { kind, .. } => kind.keyword(),
            _ => "太字",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum LineFormat {
    Indent { amount: u8, end_offset: Option<u8> },
    AlignEnd { offset: u8 },
    Center { page: bool },
    Gothic,
    FontSizeAbsolute { size: AbsoluteSize, bold: bool },
}

impl LineFormat {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn format(self) -> Format {
        match self {
            Self::Indent { .. } => Format::Indent,
            Self::AlignEnd { .. } => Format::AlignEnd,
            Self::Center { .. } => Format::Center,
            Self::Gothic => Format::Gothic,
            Self::FontSizeAbsolute { size, .. } => Format::FontSizeAbsolute(size),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RegionFormat {
    Bold {
        padded: bool,
    },
    Gothic {
        padded: bool,
    },
    Italic {
        padded: bool,
    },
    Caption {
        padded: bool,
    },
    Heading {
        level: HeadingKind,
        style: HeadingStyle,
        padded: bool,
    },
    Bouten {
        kind: BoutenKind,
        position: BoutenPosition,
    },
    SmallScript(BoutenPosition),
    Indent(IndentBlock),
    AlignEnd {
        offset: u8,
    },
    LineWidth(LineWidth),
    Table,
    Columns(ColumnCount),
    Horizontal,
    FontSize(FontShift),
    Framed(EnclosureKind),
    Warichu,
}

impl RegionFormat {
    #[must_use]
    pub const fn format(self) -> Format {
        match self {
            Self::Bold { .. } => Format::Bold,
            Self::Gothic { .. } => Format::Gothic,
            Self::Italic { .. } => Format::Italic,
            Self::Caption { .. } => Format::Caption,
            Self::Heading { level, style, .. } => Format::Heading { level, style },
            Self::Bouten { kind, .. } => Format::Bouten(kind),
            Self::SmallScript(p) => Format::SmallScript(p),
            Self::Indent(_) => Format::Indent,
            Self::AlignEnd { .. } => Format::AlignEnd,
            Self::LineWidth(_) => Format::LineWidth,
            Self::Table => Format::Table,
            Self::Columns(c) => Format::Columns(c),
            Self::Horizontal => Format::Horizontal,
            Self::FontSize(f) => Format::FontSize(f),
            Self::Framed(k) => Format::Framed(k),
            Self::Warichu => Format::Warichu,
        }
    }

    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Indent(_) => "indent",
            Self::Warichu => "warichu",
            Self::Framed(_) => "framed",
            Self::AlignEnd { .. } => "alignEnd",
            Self::LineWidth(_) => "lineWidth",
            Self::Bouten { .. } => "boutenRange",
            Self::Bold { .. } => "bold",
            Self::Gothic { .. } => "gothic",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns(_) => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "fontSize",
            Self::SmallScript(_) => "smallScript",
            Self::Caption { .. } => "caption",
        }
    }

    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Indent(_) => "indent",
            Self::Warichu => "warichu",
            Self::Framed(_) => "framed",
            Self::AlignEnd { .. } => "align-end",
            Self::LineWidth(_) => "line-width",
            Self::Bouten { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Gothic { .. } => "gothic",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns(_) => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "font-size",
            Self::SmallScript(_) => "small-script",
            Self::Caption { .. } => "caption",
        }
    }

    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Bouten { .. }
                | Self::Bold { padded: false }
                | Self::Gothic { padded: false }
                | Self::Italic { padded: false }
                | Self::SmallScript(_)
                | Self::Caption { padded: false }
        )
    }

    #[must_use]
    pub const fn content_is_phrasing(self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    pub const ALL: [Self; 16] = [
        Self::Indent(IndentBlock {
            amount: 0,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        }),
        Self::Warichu,
        Self::Framed(EnclosureKind::Rule),
        Self::AlignEnd { offset: 0 },
        Self::LineWidth(LineWidth(NonZeroU8::MIN)),
        Self::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        },
        Self::Bold { padded: false },
        Self::Gothic { padded: false },
        Self::Italic { padded: false },
        Self::Heading {
            level: HeadingKind::Large,
            style: HeadingStyle::Standard,
            padded: false,
        },
        Self::Columns(ColumnCount(NonZeroU8::MIN)),
        Self::Table,
        Self::Horizontal,
        Self::FontSize(FontShift(NonZeroI8::MIN)),
        Self::SmallScript(BoutenPosition::Right),
        Self::Caption { padded: false },
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum RegionClose {
    Indent {
        kumi_width: Option<LineWidth>,
    },
    Warichu,
    Framed(EnclosureKind),
    AlignEnd,
    LineWidth,
    Bouten {
        kind: BoutenKind,
        position: BoutenPosition,
    },
    Bold {
        padded: bool,
    },
    Gothic {
        padded: bool,
    },
    Italic {
        padded: bool,
    },
    Heading {
        level: Option<HeadingKind>,
        style: HeadingStyle,
        padded: bool,
    },
    Columns,
    Table,
    Horizontal,
    FontSize {
        larger: bool,
    },
    SmallScript(BoutenPosition),
    Caption {
        padded: bool,
    },
}

impl RegionClose {
    #[must_use]
    pub(crate) const fn of(region: RegionFormat) -> Self {
        match region {
            RegionFormat::Indent(block) => Self::Indent {
                kumi_width: match block.layout {
                    IndentLayout::Kumi(kumi) => Some(LineWidth(kumi.width)),
                    IndentLayout::LineWidth(_) | IndentLayout::None => None,
                },
            },
            RegionFormat::Warichu => Self::Warichu,
            RegionFormat::Framed(k) => Self::Framed(k),
            RegionFormat::AlignEnd { .. } => Self::AlignEnd,
            RegionFormat::LineWidth(_) => Self::LineWidth,
            RegionFormat::Bouten { kind, position } => Self::Bouten { kind, position },
            RegionFormat::Bold { padded } => Self::Bold { padded },
            RegionFormat::Gothic { padded } => Self::Gothic { padded },
            RegionFormat::Italic { padded } => Self::Italic { padded },
            RegionFormat::Heading {
                level,
                style,
                padded,
            } => Self::Heading {
                level: Some(level),
                style,
                padded,
            },
            RegionFormat::Columns(_) => Self::Columns,
            RegionFormat::Table => Self::Table,
            RegionFormat::Horizontal => Self::Horizontal,
            RegionFormat::FontSize(shift) => Self::FontSize {
                larger: shift.larger(),
            },
            RegionFormat::SmallScript(side) => Self::SmallScript(side),
            RegionFormat::Caption { padded } => Self::Caption { padded },
        }
    }

    #[must_use]
    pub(crate) const fn kind_str(self) -> &'static str {
        match self {
            Self::Indent { .. } => "indent",
            Self::Warichu => "warichu",
            Self::Framed(_) => "framed",
            Self::AlignEnd => "align-end",
            Self::LineWidth => "line-width",
            Self::Bouten { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Gothic { .. } => "gothic",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize { .. } => "font-size",
            Self::SmallScript(_) => "small-script",
            Self::Caption { .. } => "caption",
        }
    }

    #[must_use]
    pub(crate) const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Bouten { .. }
                | Self::Bold { padded: false }
                | Self::Gothic { padded: false }
                | Self::Italic { padded: false }
                | Self::SmallScript(_)
                | Self::Caption { padded: false }
        )
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn content_is_phrasing(self) -> bool {
        matches!(self, Self::Heading { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_format_is_copy_and_small() {
        const fn assert_copy<T: Copy>() {}
        assert_copy::<RegionFormat>();
        assert_copy::<Format>();
        assert_copy::<ForwardAttr>();
        assert_copy::<LineFormat>();
        assert_copy::<RegionClose>();
        assert!(size_of::<RegionFormat>() <= 12);
    }

    #[test]
    fn region_format_wire_tags_are_stable() {
        assert_eq!(
            RegionFormat::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .as_json_tag(),
            "boutenRange"
        );
        assert_eq!(
            RegionFormat::LineWidth(LineWidth(NonZeroU8::MIN)).as_json_tag(),
            "lineWidth"
        );
        assert_eq!(RegionFormat::Bold { padded: true }.as_json_tag(), "bold");
        assert_eq!(RegionFormat::Bold { padded: false }.as_json_tag(), "bold");
        assert_eq!(
            RegionFormat::Gothic { padded: true }.as_json_tag(),
            "gothic"
        );
    }

    #[test]
    fn region_close_of_round_trips_kind_str() {
        for open in RegionFormat::ALL {
            assert_eq!(
                RegionClose::of(open).kind_str(),
                open.kind_str(),
                "close family must mirror open family for {open:?}"
            );
        }
    }

    #[test]
    fn region_close_preserves_bouten_kind() {
        let line = RegionFormat::Bouten {
            kind: BoutenKind::UnderLine,
            position: BoutenPosition::Left,
        };
        assert_eq!(
            RegionClose::of(line),
            RegionClose::Bouten {
                kind: BoutenKind::UnderLine,
                position: BoutenPosition::Left,
            }
        );
    }

    #[test]
    fn region_close_indent_keeps_kumi_width() {
        let kumi = RegionFormat::Indent(IndentBlock {
            amount: 2,
            wrap: None,
            center: false,
            layout: IndentLayout::Kumi(Kumi {
                lines: NonZeroU8::MIN,
                width: NonZeroU8::new(20).unwrap(),
            }),
            styles: BlockStyles::EMPTY,
        });
        assert_eq!(
            RegionClose::of(kumi),
            RegionClose::Indent {
                kumi_width: Some(LineWidth(NonZeroU8::new(20).unwrap())),
            }
        );
        let plain = RegionFormat::Indent(IndentBlock {
            amount: 2,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        });
        assert_eq!(
            RegionClose::of(plain),
            RegionClose::Indent { kumi_width: None }
        );
    }

    #[test]
    fn format_attribute_tags() {
        assert_eq!(Format::Bouten(BoutenKind::Goma).as_json_tag(), "bouten");
        assert_eq!(Format::CombineUpright.as_json_tag(), "combineUpright");
        assert_eq!(Format::Warichu.as_json_tag(), "warichu");
        assert_eq!(Format::Center.as_json_tag(), "center");
    }

    #[test]
    fn font_shift_larger_and_magnitude() {
        let up = FontShift(NonZeroI8::new(3).unwrap());
        let down = FontShift(NonZeroI8::new(-2).unwrap());
        assert!(up.larger(), "a positive shift enlarges");
        assert!(!down.larger(), "a negative shift shrinks");
        assert!(
            FontShift(NonZeroI8::new(1).unwrap()).larger(),
            "the smallest positive step still enlarges"
        );
        assert!(
            !FontShift(NonZeroI8::new(-1).unwrap()).larger(),
            "the smallest negative step still shrinks"
        );
        assert_eq!(up.magnitude(), 3);
        assert_eq!(down.magnitude(), 2, "magnitude drops the sign");
        assert_eq!(FontShift(NonZeroI8::new(1).unwrap()).magnitude(), 1);
    }

    #[test]
    fn block_styles_is_empty_and_iter_formats() {
        assert!(BlockStyles::EMPTY.is_empty());
        assert!(BlockStyles::default().is_empty());
        let decorated = BlockStyles {
            gothic: true,
            horizontal: false,
            framed: true,
            font: Some(FontShift(NonZeroI8::new(-1).unwrap())),
        };
        assert!(!decorated.is_empty());
        assert_eq!(
            decorated.iter_formats().collect::<Vec<_>>(),
            vec![
                Format::Gothic,
                Format::Framed(EnclosureKind::Rule),
                Format::FontSize(FontShift(NonZeroI8::new(-1).unwrap())),
            ],
            "iter_formats must project set members in canonical order"
        );
        assert_eq!(BlockStyles::EMPTY.iter_formats().count(), 0);
    }

    #[test]
    fn forward_origin_from_consume_pins_the_pullback_boundary() {
        assert_eq!(ForwardOrigin::from_consume(5, 10), ForwardOrigin::Reclaimed);
        assert_eq!(
            ForwardOrigin::from_consume(10, 10),
            ForwardOrigin::Referenced,
            "equal offsets mean no pull-back → Referenced"
        );
        assert_eq!(
            ForwardOrigin::from_consume(15, 10),
            ForwardOrigin::Referenced
        );
    }

    #[test]
    fn region_inline_and_phrasing_predicates() {
        assert!(
            RegionFormat::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .is_inline()
        );
        assert!(RegionFormat::Bold { padded: false }.is_inline());
        assert!(RegionFormat::SmallScript(BoutenPosition::Right).is_inline());
        assert!(!RegionFormat::Bold { padded: true }.is_inline());
        assert!(!RegionFormat::Table.is_inline());
        assert!(!RegionFormat::Warichu.is_inline());
        assert!(
            RegionFormat::Heading {
                level: HeadingKind::Large,
                style: HeadingStyle::Standard,
                padded: true,
            }
            .content_is_phrasing()
        );
        assert!(!RegionFormat::Table.content_is_phrasing());
        assert!(!RegionFormat::Bold { padded: false }.content_is_phrasing());
    }

    #[test]
    fn region_close_inline_and_phrasing_predicates() {
        assert!(
            RegionClose::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .is_inline()
        );
        assert!(RegionClose::Bold { padded: false }.is_inline());
        assert!(RegionClose::SmallScript(BoutenPosition::Right).is_inline());
        assert!(!RegionClose::Bold { padded: true }.is_inline());
        assert!(!RegionClose::Table.is_inline());
        assert!(!RegionClose::Warichu.is_inline());
        assert!(
            RegionClose::Heading {
                level: Some(HeadingKind::Large),
                style: HeadingStyle::Standard,
                padded: true,
            }
            .content_is_phrasing()
        );
        assert!(!RegionClose::Table.content_is_phrasing());
        assert!(!RegionClose::Bold { padded: false }.content_is_phrasing());
    }

    #[test]
    fn scope_projections_are_total() {
        assert_eq!(ForwardAttr::Bold.format(), Format::Bold);
        assert_eq!(LineFormat::Gothic.format(), Format::Gothic);
        assert_eq!(
            RegionFormat::Framed(EnclosureKind::Rule).format(),
            Format::Framed(EnclosureKind::Rule),
            "the enclosure identity is reached via the live block / forward scopes"
        );
        assert_eq!(RegionFormat::Warichu.format(), Format::Warichu);
        assert_eq!(
            RegionFormat::Bold { padded: true }.format(),
            Format::Bold,
            "range and forward Bold share the attribute identity"
        );
    }
}
