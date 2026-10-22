#![expect(
    clippy::expect_used,
    reason = "the static directive pattern set is non-empty and valid"
)]

//! Classifies annotation bodies through [`classify_annotation_body`].
//! Keeps unsupported notation verbatim under ADR-0027.

use std::sync::OnceLock;

/// This limit covers the measured ninety-ninth percentile of corpus okurigana lengths.
const MAX_OKURIGANA_CHARS: usize = 6;
const OKURIGANA_CHAR_BYTES: usize = 3;
const MAX_OKURIGANA_BYTES: usize = MAX_OKURIGANA_CHARS * OKURIGANA_CHAR_BYTES;
const DECIMAL_RADIX: u32 = 10;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, Anchored, Input, MatchKind, StartKind};
use core::num::{NonZeroI8, NonZeroU8};

use crate::syntax::alloc::Allocator;
use crate::syntax::ast::Directive;
use crate::syntax::{
    AbsoluteSize, BOUTEN_KINDS, BlockStyles, BoutenKind, BoutenPosition, ColumnCount,
    DirectiveKind, EnclosureKind, FontShift, HeadingKind, HeadingStyle, IndentBlock, IndentLayout,
    Kumi, LineFormat, LineWidth, RegionClose, RegionFormat, SectionKind,
};

use super::EmitKind;

#[derive(Clone, Copy)]
struct BodyPattern {
    needle: &'static str,
    family: BodyFamily,
}

#[derive(Clone, Copy)]
enum BodyFamily {
    PageBreak,
    BodyEnd,
    ForcedBreak,
    SectionKaicho,
    SectionKaidan,
    SectionKaimihiraki,
    AlignEnd0,
    CenterMarker,
    LineGothic,
    KeigakomiOpen,
    KeigakomiClose,
    IndentBlock1,
    PageCenterBlockOpen,
    AlignEndBlock0,
    IndentBlockEnd,
    AlignEndBlockEnd,
    LineWidthBlockEnd,
    TableBlockOpen,
    TableBlockEnd,
    HorizontalBlockOpen,
    HorizontalBlockEnd,
    FontSizeBlockEnd,
    ColumnsBlockEnd,
    WarichuOpen,
    WarichuClose,
    KaeritenSingle,
    KaeritenCompound,

    AlignEndParamPrefix,
    SashiePrefix,
    IndentBlockParamPrefix,
    AlignEndBlockParamPrefix,
    IndentKumiBlockEnd,
    OkuriganaPrefix,

    IndentParamPrefix,

    BoutenRange,

    Emphasis,

    SmallScriptRange,

    CaptionRange,

    WarichuBlockOpen,
    WarichuBlockEnd,
    TopIndentPrefix,
    KaigyouTentsukiPrefix,

    LineFontSize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Exact,
    Prefix,
    Reparse,
}

const fn body_family_mode(family: BodyFamily) -> MatchMode {
    match family {
        BodyFamily::PageBreak
        | BodyFamily::BodyEnd
        | BodyFamily::ForcedBreak
        | BodyFamily::SectionKaicho
        | BodyFamily::SectionKaidan
        | BodyFamily::SectionKaimihiraki
        | BodyFamily::AlignEnd0
        | BodyFamily::CenterMarker
        | BodyFamily::LineGothic
        | BodyFamily::KeigakomiOpen
        | BodyFamily::KeigakomiClose
        | BodyFamily::WarichuBlockOpen
        | BodyFamily::WarichuBlockEnd
        | BodyFamily::IndentBlock1
        | BodyFamily::PageCenterBlockOpen
        | BodyFamily::AlignEndBlock0
        | BodyFamily::AlignEndBlockEnd
        | BodyFamily::LineWidthBlockEnd
        | BodyFamily::TableBlockOpen
        | BodyFamily::TableBlockEnd
        | BodyFamily::HorizontalBlockOpen
        | BodyFamily::HorizontalBlockEnd
        | BodyFamily::FontSizeBlockEnd
        | BodyFamily::WarichuOpen
        | BodyFamily::WarichuClose
        | BodyFamily::KaeritenSingle
        | BodyFamily::KaeritenCompound => MatchMode::Exact,
        BodyFamily::AlignEndParamPrefix
        | BodyFamily::SashiePrefix
        | BodyFamily::IndentBlockParamPrefix
        | BodyFamily::AlignEndBlockParamPrefix
        | BodyFamily::IndentBlockEnd
        | BodyFamily::IndentKumiBlockEnd
        | BodyFamily::ColumnsBlockEnd
        | BodyFamily::OkuriganaPrefix
        | BodyFamily::TopIndentPrefix
        | BodyFamily::KaigyouTentsukiPrefix => MatchMode::Prefix,
        BodyFamily::IndentParamPrefix
        | BodyFamily::BoutenRange
        | BodyFamily::Emphasis
        | BodyFamily::SmallScriptRange
        | BodyFamily::CaptionRange
        | BodyFamily::LineFontSize => MatchMode::Reparse,
    }
}

static BODY_PATTERNS: &[BodyPattern] = &[
    BodyPattern {
        needle: "ここから字下げ",
        family: BodyFamily::IndentBlock1,
    },
    BodyPattern {
        needle: "ここからページの左右中央",
        family: BodyFamily::PageCenterBlockOpen,
    },
    BodyPattern {
        needle: "ここから地付き",
        family: BodyFamily::AlignEndBlock0,
    },
    BodyPattern {
        needle: "ここから地から",
        family: BodyFamily::AlignEndBlockParamPrefix,
    },
    BodyPattern {
        needle: "ここから",
        family: BodyFamily::IndentBlockParamPrefix,
    },
    BodyPattern {
        needle: "ここで字下げ終わり",
        family: BodyFamily::IndentBlockEnd,
    },
    BodyPattern {
        needle: "ここで字下げ、",
        family: BodyFamily::IndentKumiBlockEnd,
    },
    BodyPattern {
        needle: "ここで地付き終わり",
        family: BodyFamily::AlignEndBlockEnd,
    },
    BodyPattern {
        needle: "ここで字上げ終わり",
        family: BodyFamily::AlignEndBlockEnd,
    },
    BodyPattern {
        needle: "ここで字詰め終わり",
        family: BodyFamily::LineWidthBlockEnd,
    },
    BodyPattern {
        needle: "ここから表",
        family: BodyFamily::TableBlockOpen,
    },
    BodyPattern {
        needle: "ここで表終わり",
        family: BodyFamily::TableBlockEnd,
    },
    BodyPattern {
        needle: "ここから横組み",
        family: BodyFamily::HorizontalBlockOpen,
    },
    BodyPattern {
        needle: "ここで横組み終わり",
        family: BodyFamily::HorizontalBlockEnd,
    },
    BodyPattern {
        needle: "ここで大きな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    BodyPattern {
        needle: "ここで小さな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    BodyPattern {
        needle: "大きな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    BodyPattern {
        needle: "小さな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    BodyPattern {
        needle: "横組み",
        family: BodyFamily::HorizontalBlockOpen,
    },
    BodyPattern {
        needle: "横組み終わり",
        family: BodyFamily::HorizontalBlockEnd,
    },
    BodyPattern {
        needle: "行右小書き",
        family: BodyFamily::SmallScriptRange,
    },
    BodyPattern {
        needle: "行右小書き終わり",
        family: BodyFamily::SmallScriptRange,
    },
    BodyPattern {
        needle: "行左小書き",
        family: BodyFamily::SmallScriptRange,
    },
    BodyPattern {
        needle: "行左小書き終わり",
        family: BodyFamily::SmallScriptRange,
    },
    BodyPattern {
        needle: "キャプション",
        family: BodyFamily::CaptionRange,
    },
    BodyPattern {
        needle: "キャプション終わり",
        family: BodyFamily::CaptionRange,
    },
    BodyPattern {
        needle: "ここからキャプション",
        family: BodyFamily::CaptionRange,
    },
    BodyPattern {
        needle: "ここでキャプション終わり",
        family: BodyFamily::CaptionRange,
    },
    BodyPattern {
        needle: "ここから罫囲み",
        family: BodyFamily::KeigakomiOpen,
    },
    BodyPattern {
        needle: "ここで罫囲み終わり",
        family: BodyFamily::KeigakomiClose,
    },
    BodyPattern {
        needle: "ここから割り注",
        family: BodyFamily::WarichuBlockOpen,
    },
    BodyPattern {
        needle: "ここで割り注終わり",
        family: BodyFamily::WarichuBlockEnd,
    },
    BodyPattern {
        needle: "天から",
        family: BodyFamily::TopIndentPrefix,
    },
    BodyPattern {
        needle: "天より",
        family: BodyFamily::TopIndentPrefix,
    },
    BodyPattern {
        needle: "改行天付き",
        family: BodyFamily::KaigyouTentsukiPrefix,
    },
    BodyPattern {
        needle: "ここで",
        family: BodyFamily::ColumnsBlockEnd,
    },
    BodyPattern {
        needle: "改ページ",
        family: BodyFamily::PageBreak,
    },
    BodyPattern {
        needle: "改頁",
        family: BodyFamily::PageBreak,
    },
    BodyPattern {
        needle: "改丁",
        family: BodyFamily::SectionKaicho,
    },
    BodyPattern {
        needle: "改段",
        family: BodyFamily::SectionKaidan,
    },
    BodyPattern {
        needle: "改見開き",
        family: BodyFamily::SectionKaimihiraki,
    },
    BodyPattern {
        needle: "本文終わり",
        family: BodyFamily::BodyEnd,
    },
    BodyPattern {
        needle: "改行",
        family: BodyFamily::ForcedBreak,
    },
    BodyPattern {
        needle: "地から",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "地より",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "ここから地より",
        family: BodyFamily::AlignEndBlockParamPrefix,
    },
    BodyPattern {
        needle: "文末より",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "行末より",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "この行は行末より",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "地付き",
        family: BodyFamily::AlignEnd0,
    },
    BodyPattern {
        needle: "右寄せ",
        family: BodyFamily::AlignEnd0,
    },
    BodyPattern {
        needle: "地寄せ",
        family: BodyFamily::AlignEnd0,
    },
    BodyPattern {
        needle: "ページの左右中央",
        family: BodyFamily::CenterMarker,
    },
    BodyPattern {
        needle: "中央揃え",
        family: BodyFamily::CenterMarker,
    },
    BodyPattern {
        needle: "この行はゴシック体",
        family: BodyFamily::LineGothic,
    },
    BodyPattern {
        needle: "特大文字",
        family: BodyFamily::LineFontSize,
    },
    BodyPattern {
        needle: "大文字",
        family: BodyFamily::LineFontSize,
    },
    BodyPattern {
        needle: "中文字",
        family: BodyFamily::LineFontSize,
    },
    BodyPattern {
        needle: "小文字",
        family: BodyFamily::LineFontSize,
    },
    BodyPattern {
        needle: "挿絵",
        family: BodyFamily::SashiePrefix,
    },
    BodyPattern {
        needle: "罫囲み終わり",
        family: BodyFamily::KeigakomiClose,
    },
    BodyPattern {
        needle: "罫囲み",
        family: BodyFamily::KeigakomiOpen,
    },
    BodyPattern {
        needle: "割り注終わり",
        family: BodyFamily::WarichuClose,
    },
    BodyPattern {
        needle: "割り注",
        family: BodyFamily::WarichuOpen,
    },
    BodyPattern {
        needle: "左に",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白ゴマ傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "二重丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "蛇の目傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "ばつ傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白三角傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "黒三角傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "二重傍線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "鎖線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "破線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "波線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "傍線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "太字",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "斜体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここから太字",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここから斜体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここで太字終わり",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここで斜体終わり",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ゴシック体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここからゴシック体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここでゴシック体終わり",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "（",
        family: BodyFamily::OkuriganaPrefix,
    },
    BodyPattern {
        needle: "一レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "上レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "下レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "中レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "二レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "三レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "上二",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "下二",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "一",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "丁",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "三",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "上",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "下",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "中",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "丙",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "乙",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "二",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "四",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "甲",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "レ",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "0",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "1",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "2",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "3",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "4",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "5",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "6",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "7",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "8",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "9",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "０",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "１",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "２",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "３",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "４",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "５",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "６",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "７",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "８",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "９",
        family: BodyFamily::IndentParamPrefix,
    },
];

#[must_use]
pub(crate) fn build_body_dispatcher() -> AhoCorasick {
    AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .start_kind(StartKind::Anchored)
        .build(BODY_PATTERNS.iter().map(|p| p.needle))
        .expect("BODY_PATTERNS is a static, non-empty, valid set")
}

static BODY_DISPATCHER: OnceLock<AhoCorasick> = OnceLock::new();

fn body_dispatcher() -> &'static AhoCorasick {
    BODY_DISPATCHER.get_or_init(build_body_dispatcher)
}

pub(crate) fn prewarm() -> &'static AhoCorasick {
    body_dispatcher()
}

pub(super) fn editorial_note_kind(body: &str) -> Option<DirectiveKind> {
    if body == "ママ" || body.ends_with("はママ") || body == "底本のまま" {
        Some(DirectiveKind::Sic)
    } else if body.contains("底本では") || body.contains("初出では") {
        Some(DirectiveKind::BaseTextVariant)
    } else if is_editor_note_body(body) {
        Some(DirectiveKind::EditorNote)
    } else if is_ruby_attached_body(body) {
        Some(DirectiveKind::RubyAttached)
    } else if is_ruby_retarget_body(body) {
        Some(DirectiveKind::RubyRetarget)
    } else if body == "左にルビ付き" {
        Some(DirectiveKind::RubyPairOpen)
    } else if is_ruby_pair_close_body(body) {
        Some(DirectiveKind::RubyPairClose)
    } else if body == "注記付き" || body == "左に注記付き" {
        Some(DirectiveKind::MarginNotePairOpen)
    } else if is_margin_note_pair_close_body(body) {
        Some(DirectiveKind::MarginNotePairClose)
    } else {
        None
    }
}

fn is_ruby_attached_body(body: &str) -> bool {
    body.strip_prefix("「")
        .and_then(|r| r.strip_suffix("」にルビ"))
        .is_some_and(|x| !x.is_empty())
}

fn is_ruby_retarget_body(body: &str) -> bool {
    body.strip_prefix("ルビは「")
        .and_then(|r| r.strip_suffix("」にかかる"))
        .is_some_and(|x| !x.is_empty())
}

fn is_ruby_pair_close_body(body: &str) -> bool {
    body.strip_prefix("左に「")
        .and_then(|r| r.strip_suffix("」のルビ付き終わり"))
        .is_some_and(|y| !y.is_empty())
}

fn is_margin_note_pair_close_body(body: &str) -> bool {
    body.strip_prefix("左に「")
        .or_else(|| body.strip_prefix("「"))
        .and_then(|r| r.strip_suffix("」の注記付き終わり"))
        .is_some_and(|y| !y.is_empty())
}

fn is_editor_note_body(body: &str) -> bool {
    let Some(rest) = body.strip_prefix("入力者注(") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(')') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

#[expect(
    clippy::too_many_lines,
    reason = "single match arm per BodyFamily — splitting would scatter \
              the dispatch logic and obscure the intentional 1:1 mapping"
)]
pub(super) fn classify_annotation_body(
    body: &str,
    alloc: &mut Allocator,
) -> Option<(EmitKind, Option<Directive>)> {
    if body.is_empty() {
        return None;
    }
    if let Some(emit) = parse_heading_directive(body) {
        return Some((emit, None));
    }
    let dfa = body_dispatcher();
    let mat = dfa.find(Input::new(body).anchored(Anchored::Yes))?;
    let pat = BODY_PATTERNS[mat.pattern().as_usize()];
    let match_end = mat.end();
    let exact = match_end == body.len();
    if body_family_mode(pat.family) == MatchMode::Exact && !exact {
        return None;
    }
    match pat.family {
        BodyFamily::PageBreak => Some((EmitKind::Aozora(alloc.page_break()), None)),
        BodyFamily::BodyEnd => Some((EmitKind::Aozora(alloc.body_end()), None)),
        BodyFamily::ForcedBreak => Some((EmitKind::Aozora(alloc.forced_break()), None)),
        BodyFamily::SectionKaicho => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaicho)),
            None,
        )),
        BodyFamily::SectionKaidan => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaidan)),
            None,
        )),
        BodyFamily::SectionKaimihiraki => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaimihiraki)),
            None,
        )),
        BodyFamily::AlignEnd0 => Some((
            EmitKind::Aozora(alloc.line(LineFormat::AlignEnd { offset: 0 })),
            None,
        )),
        BodyFamily::CenterMarker => {
            let page = body == "ページの左右中央";
            Some((
                EmitKind::Aozora(alloc.line(LineFormat::Center { page })),
                None,
            ))
        }
        BodyFamily::LineGothic => Some((EmitKind::Aozora(alloc.line(LineFormat::Gothic)), None)),
        BodyFamily::LineFontSize => parse_line_font_size(body).map(|(size, bold)| {
            (
                EmitKind::Aozora(alloc.line(LineFormat::FontSizeAbsolute { size, bold })),
                None,
            )
        }),
        BodyFamily::KeigakomiOpen => Some((
            EmitKind::BlockOpen(RegionFormat::Framed(EnclosureKind::Rule)),
            None,
        )),
        BodyFamily::KeigakomiClose => Some((
            EmitKind::BlockClose(RegionClose::Framed(EnclosureKind::Rule)),
            None,
        )),
        BodyFamily::WarichuBlockOpen => Some((EmitKind::BlockOpen(RegionFormat::Warichu), None)),
        BodyFamily::WarichuBlockEnd => Some((EmitKind::BlockClose(RegionClose::Warichu), None)),
        BodyFamily::IndentBlock1 => Some((
            EmitKind::BlockOpen(RegionFormat::Indent(IndentBlock {
                amount: 1,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            })),
            None,
        )),
        BodyFamily::PageCenterBlockOpen => Some((
            EmitKind::BlockOpen(RegionFormat::Indent(IndentBlock {
                amount: 0,
                wrap: None,
                center: true,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            })),
            None,
        )),
        BodyFamily::AlignEndBlock0 => Some((
            EmitKind::BlockOpen(RegionFormat::AlignEnd { offset: 0 }),
            None,
        )),
        BodyFamily::IndentBlockEnd => {
            let tail = &body[match_end..];
            (tail.is_empty() || (tail.starts_with('、') && tail.ends_with("終わり"))).then_some((
                EmitKind::BlockClose(RegionClose::Indent { kumi_width: None }),
                None,
            ))
        }
        BodyFamily::AlignEndBlockEnd => Some((EmitKind::BlockClose(RegionClose::AlignEnd), None)),
        BodyFamily::LineWidthBlockEnd => Some((EmitKind::BlockClose(RegionClose::LineWidth), None)),
        BodyFamily::IndentKumiBlockEnd => {
            let rest = &body[match_end..];
            let rest = rest.split_once('行').map_or(rest, |(_lines, after)| after);
            let (width, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字組み終わり")
                .then(|| NonZeroU8::new(width))
                .flatten()
                .map(|w| {
                    (
                        EmitKind::BlockClose(RegionClose::Indent {
                            kumi_width: Some(LineWidth(w)),
                        }),
                        None,
                    )
                })
        }
        BodyFamily::TableBlockOpen => Some((EmitKind::BlockOpen(RegionFormat::Table), None)),
        BodyFamily::TableBlockEnd => Some((EmitKind::BlockClose(RegionClose::Table), None)),
        BodyFamily::HorizontalBlockOpen => {
            Some((EmitKind::BlockOpen(RegionFormat::Horizontal), None))
        }
        BodyFamily::HorizontalBlockEnd => {
            Some((EmitKind::BlockClose(RegionClose::Horizontal), None))
        }
        BodyFamily::FontSizeBlockEnd => {
            let larger = !body.contains("小さな");
            Some((EmitKind::BlockClose(RegionClose::FontSize { larger }), None))
        }
        BodyFamily::ColumnsBlockEnd => {
            let rest = &body[match_end..];
            let rest = parse_decimal_u8_prefix(rest).map_or(rest, |(_n, tail)| tail);
            (rest == "段組終わり" || rest == "段組み終わり")
                .then_some((EmitKind::BlockClose(RegionClose::Columns), None))
        }
        BodyFamily::WarichuOpen => {
            let p = alloc.make_directive("［＃割り注］", DirectiveKind::WarichuOpen);
            let node = alloc.annotation(p);
            let p2 = alloc.make_directive("［＃割り注］", DirectiveKind::WarichuOpen);
            Some((EmitKind::Aozora(node), Some(p2)))
        }
        BodyFamily::WarichuClose => {
            let p = alloc.make_directive("［＃割り注終わり］", DirectiveKind::WarichuClose);
            let node = alloc.annotation(p);
            let p2 = alloc.make_directive("［＃割り注終わり］", DirectiveKind::WarichuClose);
            Some((EmitKind::Aozora(node), Some(p2)))
        }
        BodyFamily::KaeritenSingle | BodyFamily::KaeritenCompound => {
            Some((EmitKind::Aozora(alloc.kaeriten(body)), None))
        }

        BodyFamily::AlignEndParamPrefix => {
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (matches!(tail, "字上げ" | "字上がり" | "字上げ揃え" | "字上がり揃え") && n >= 1).then(
                || {
                    (
                        EmitKind::Aozora(alloc.line(LineFormat::AlignEnd { offset: n })),
                        None,
                    )
                },
            )
        }
        BodyFamily::TopIndentPrefix => {
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            if tail == "字下げ" && n >= 1 {
                Some((
                    EmitKind::Aozora(alloc.line(LineFormat::Indent {
                        amount: n,
                        end_offset: None,
                    })),
                    None,
                ))
            } else {
                parse_both_margin_tail(n, tail).map(|lf| (EmitKind::Aozora(alloc.line(lf)), None))
            }
        }
        BodyFamily::KaigyouTentsukiPrefix => {
            let rest = &body[match_end..];
            let after = rest.strip_prefix("、折り返して")?;
            let (m, tail) = parse_decimal_u8_prefix(after)?;
            (tail == "字下げ").then_some((
                EmitKind::BlockOpen(RegionFormat::Indent(IndentBlock {
                    amount: 0,
                    wrap: Some(m),
                    center: false,
                    layout: IndentLayout::None,
                    styles: BlockStyles::EMPTY,
                })),
                None,
            ))
        }
        BodyFamily::SashiePrefix => classify_sashie_body(body, alloc).map(|e| (e, None)),
        BodyFamily::IndentBlockParamPrefix => {
            let rest = &body[match_end..];
            if let Some(after) = rest
                .strip_prefix("改行天付き、折り返して")
                .or_else(|| rest.strip_prefix("天付き、折り返して"))
            {
                let (m, tail2) = parse_decimal_u8_prefix(after)?;
                return (tail2 == "字下げ").then_some((
                    EmitKind::BlockOpen(RegionFormat::Indent(IndentBlock {
                        amount: 0,
                        wrap: Some(m),
                        center: false,
                        layout: IndentLayout::None,
                        styles: BlockStyles::EMPTY,
                    })),
                    None,
                ));
            }
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            if tail == "字下げ" {
                Some((
                    EmitKind::BlockOpen(RegionFormat::Indent(IndentBlock {
                        amount: n,
                        wrap: None,
                        center: false,
                        layout: IndentLayout::None,
                        styles: BlockStyles::EMPTY,
                    })),
                    None,
                ))
            } else if let Some(after) = tail.strip_prefix("字下げ、") {
                parse_indent_compound(n, after)
                    .map(|block| (EmitKind::BlockOpen(RegionFormat::Indent(block)), None))
            } else if tail == "字詰め" {
                NonZeroU8::new(n).map(|w| {
                    (
                        EmitKind::BlockOpen(RegionFormat::LineWidth(LineWidth(w))),
                        None,
                    )
                })
            } else if tail == "段組" || tail == "段組み" {
                NonZeroU8::new(n).map(|c| {
                    (
                        EmitKind::BlockOpen(RegionFormat::Columns(ColumnCount(c))),
                        None,
                    )
                })
            } else {
                font_size_block_open_steps(tail, n)
                    .and_then(NonZeroI8::new)
                    .map(|s| {
                        (
                            EmitKind::BlockOpen(RegionFormat::FontSize(FontShift(s))),
                            None,
                        )
                    })
            }
        }
        BodyFamily::AlignEndBlockParamPrefix => {
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字上げ").then_some((
                EmitKind::BlockOpen(RegionFormat::AlignEnd { offset: n }),
                None,
            ))
        }
        BodyFamily::OkuriganaPrefix => {
            is_okurigana_body(body).then(|| (EmitKind::Aozora(alloc.kaeriten(body)), None))
        }
        BodyFamily::IndentParamPrefix => {
            let (n, tail) = parse_decimal_u8_prefix(body)?;
            if tail == "字下げ" && n >= 1 {
                Some((
                    EmitKind::Aozora(alloc.line(LineFormat::Indent {
                        amount: n,
                        end_offset: None,
                    })),
                    None,
                ))
            } else if let Some(lf) = parse_both_margin_tail(n, tail) {
                Some((EmitKind::Aozora(alloc.line(lf)), None))
            } else {
                font_size_block_open_steps(tail, n)
                    .and_then(NonZeroI8::new)
                    .map(|s| {
                        (
                            EmitKind::BlockOpen(RegionFormat::FontSize(FontShift(s))),
                            None,
                        )
                    })
            }
        }
        BodyFamily::BoutenRange => {
            let (kind, position, is_close) = parse_bouten_range_body(body)?;
            Some((
                open_or_close(RegionFormat::Bouten { kind, position }, is_close),
                None,
            ))
        }
        BodyFamily::Emphasis => {
            let (weight, padded, is_close) = parse_emphasis_body(body)?;
            let region = match weight {
                EmphasisWeight::Bold => RegionFormat::Bold { padded },
                EmphasisWeight::Gothic => RegionFormat::Gothic { padded },
                EmphasisWeight::Italic => RegionFormat::Italic { padded },
            };
            Some((open_or_close(region, is_close), None))
        }
        BodyFamily::SmallScriptRange => {
            let (side, is_close) = parse_small_script_range_body(body)?;
            Some((
                open_or_close(RegionFormat::SmallScript(side), is_close),
                None,
            ))
        }
        BodyFamily::CaptionRange => {
            let (padded, is_close) = parse_caption_body(body)?;
            Some((
                open_or_close(RegionFormat::Caption { padded }, is_close),
                None,
            ))
        }
    }
}

fn open_or_close(region: RegionFormat, is_close: bool) -> EmitKind {
    if is_close {
        EmitKind::BlockClose(RegionClose::of(region))
    } else {
        EmitKind::BlockOpen(region)
    }
}

fn is_okurigana_body(body: &str) -> bool {
    let Some(inner) = body.strip_prefix('（').and_then(|s| s.strip_suffix('）')) else {
        return false;
    };
    if !(OKURIGANA_CHAR_BYTES..=MAX_OKURIGANA_BYTES).contains(&inner.len()) {
        return false;
    }
    let mut count = 0usize;
    for c in inner.chars() {
        count += 1;
        if count > MAX_OKURIGANA_CHARS || !is_okurigana_char(c) {
            return false;
        }
    }
    count >= 1
}

const fn is_okurigana_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3041}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{FF66}'..='\u{FF9F}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
    )
}

fn classify_sashie_body(body: &str, alloc: &mut Allocator) -> Option<EmitKind> {
    let after_kw = body.strip_prefix("挿絵")?;
    let paren = after_kw.find('（')?;
    let number = if paren == 0 {
        None
    } else {
        let num = &after_kw[..paren];
        if num
            .chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
        {
            Some(num)
        } else {
            return None;
        }
    };
    let rest = &after_kw[paren + '（'.len_utf8()..];
    let close_off = rest.find('）')?;
    let inside = &rest[..close_off];
    let (file, dimensions) = match inside.split_once('、') {
        Some((f, dims)) if !f.is_empty() && !dims.is_empty() => (f, Some(dims)),
        _ => (inside, None),
    };
    if file.is_empty() {
        return None;
    }
    let tail = &rest[close_off + '）'.len_utf8()..];
    let caption = if tail == "入る" {
        None
    } else {
        let inner = tail
            .strip_prefix('「')
            .and_then(|t| t.strip_suffix("」入る"))?;
        if inner.is_empty() {
            return None;
        }
        Some(alloc.content_plain(inner))
    };
    Some(EmitKind::Aozora(
        alloc.sashie(file, number, dimensions, caption),
    ))
}

pub(super) fn classify_general_image_body(body: &str, alloc: &mut Allocator) -> Option<EmitKind> {
    let middle = body.strip_suffix("入る")?;
    let paren = middle.rfind('（')?;
    let description = &middle[..paren];
    if description.is_empty() {
        return None;
    }
    let rest = &middle[paren + '（'.len_utf8()..];
    let close_off = rest.find('）')?;
    if close_off + '）'.len_utf8() != rest.len() {
        return None;
    }
    let inside = &rest[..close_off];
    let (file, dimensions) = match inside.split_once('、') {
        Some((f, dims)) if !f.is_empty() && !dims.is_empty() => (f, Some(dims)),
        _ => (inside, None),
    };
    if file.is_empty() {
        return None;
    }
    Some(EmitKind::Aozora(alloc.sashie_general(
        file,
        description,
        dimensions,
    )))
}

pub(super) fn parse_heading_keyword(s: &str) -> Option<(HeadingStyle, HeadingKind)> {
    let (style, rest) = strip_heading_style(s);
    let kind = match rest {
        "大見出し" => HeadingKind::Large,
        "中見出し" => HeadingKind::Medium,
        "小見出し" => HeadingKind::Small,
        _ => return None,
    };
    Some((style, kind))
}

fn strip_heading_style(s: &str) -> (HeadingStyle, &str) {
    [
        ("同行", HeadingStyle::SameLine),
        ("窓", HeadingStyle::Window),
    ]
    .into_iter()
    .find_map(|(prefix, style)| s.strip_prefix(prefix).map(|rest| (style, rest)))
    .unwrap_or((HeadingStyle::Standard, s))
}

fn parse_heading_close_level(s: &str) -> Option<(HeadingStyle, Option<HeadingKind>)> {
    let (style, rest) = strip_heading_style(s);
    let level = match rest {
        "大見出し" => Some(HeadingKind::Large),
        "中見出し" => Some(HeadingKind::Medium),
        "小見出し" => Some(HeadingKind::Small),
        "見出し" if matches!(style, HeadingStyle::Standard) => None,
        _ => return None,
    };
    Some((style, level))
}

fn parse_heading_directive(body: &str) -> Option<EmitKind> {
    if let Some(rest) = body.strip_prefix("ここから") {
        let (style, level) = parse_heading_keyword(rest)?;
        return Some(EmitKind::BlockOpen(RegionFormat::Heading {
            level,
            style,
            padded: true,
        }));
    }
    if let Some(rest) = body.strip_prefix("ここで") {
        let (style, level) = parse_heading_close_level(rest.strip_suffix("終わり")?)?;
        return Some(EmitKind::BlockClose(RegionClose::Heading {
            level,
            style,
            padded: true,
        }));
    }
    if let Some(inner) = body.strip_suffix("終わり") {
        let (style, level) = parse_heading_close_level(inner)?;
        let level = Some(level?);
        return Some(EmitKind::BlockClose(RegionClose::Heading {
            level,
            style,
            padded: false,
        }));
    }
    let (style, level) = parse_heading_keyword(body)?;
    Some(EmitKind::BlockOpen(RegionFormat::Heading {
        level,
        style,
        padded: false,
    }))
}

fn font_size_block_open_steps(tail: &str, magnitude: u8) -> Option<i8> {
    let steps = i8::try_from(magnitude).ok()?;
    if steps == 0 {
        return None;
    }
    match tail {
        "段階大きな文字" => Some(steps),
        "段階小さな文字" => Some(-steps),
        _ => None,
    }
}

fn parse_indent_compound(amount: u8, after: &str) -> Option<IndentBlock> {
    let mut block = IndentBlock {
        amount,
        wrap: None,
        center: false,
        layout: IndentLayout::None,
        styles: BlockStyles::EMPTY,
    };
    for segment in after.split('、') {
        resolve_indent_segment(segment, &mut block)?;
    }
    Some(block)
}

fn resolve_indent_segment(segment: &str, block: &mut IndentBlock) -> Option<()> {
    if let Some(rest) = segment.strip_prefix("折り返して") {
        let (m, tail) = parse_decimal_u8_prefix(rest)?;
        if tail != "字下げ" || block.wrap.is_some() {
            return None;
        }
        block.wrap = Some(m);
        return Some(());
    }
    if matches!(
        segment,
        "ページの左右中央" | "ページの左右中央に" | "左右中央" | "中央揃え"
    ) {
        if block.center {
            return None;
        }
        block.center = true;
        return Some(());
    }
    if let Some(layout) = parse_indent_line_layout(segment) {
        if !matches!(block.layout, IndentLayout::None) {
            return None;
        }
        block.layout = layout;
        return Some(());
    }
    let styles = &mut block.styles;
    match segment {
        "ゴシック体" if !styles.gothic => styles.gothic = true,
        "横書き" | "横組み" if !styles.horizontal => styles.horizontal = true,
        "罫囲み" if !styles.framed => styles.framed = true,
        "小さい活字" if styles.font.is_none() => {
            styles.font = Some(FontShift(NonZeroI8::new(-1)?));
        }
        _ => return None,
    }
    Some(())
}

fn parse_both_margin_tail(amount: u8, tail: &str) -> Option<LineFormat> {
    let rest = tail.strip_prefix("字下げ")?;
    let rest = rest.strip_prefix('て').unwrap_or(rest);
    let rest = rest.strip_prefix('、').unwrap_or(rest);
    let rest = rest.strip_prefix("地より")?;
    let (offset, rest) = parse_decimal_u8_prefix(rest)?;
    let rest = rest
        .strip_prefix("字あき")
        .or_else(|| rest.strip_prefix("字上げ"))?;
    let rest = rest
        .strip_prefix('で')
        .or_else(|| rest.strip_prefix('て'))
        .unwrap_or(rest);
    (amount >= 1 && offset >= 1 && rest.is_empty()).then_some(LineFormat::Indent {
        amount,
        end_offset: Some(offset),
    })
}

fn parse_indent_line_layout(after: &str) -> Option<IndentLayout> {
    let (lead, rest) = parse_decimal_u8_prefix(after)?;
    let lead = NonZeroU8::new(lead)?;
    if rest == "字詰め" {
        return Some(IndentLayout::LineWidth(LineWidth(lead)));
    }
    let after_lines = rest.strip_prefix('行')?;
    let (width, tail) = parse_decimal_u8_prefix(after_lines)?;
    let width = NonZeroU8::new(width)?;
    if matches!(tail, "字組み" | "字組みで") {
        return Some(IndentLayout::Kumi(Kumi { lines: lead, width }));
    }
    None
}

pub(super) fn bouten_kind_from_suffix(s: &str) -> Option<BoutenKind> {
    if s == "×傍点" {
        return Some(BoutenKind::Cross);
    }
    BOUTEN_KINDS.iter().copied().find(|k| k.keyword() == s)
}

fn parse_bouten_range_body(body: &str) -> Option<(BoutenKind, BoutenPosition, bool)> {
    let (position, rest) = body
        .strip_prefix("左に")
        .map_or((BoutenPosition::Right, body), |r| (BoutenPosition::Left, r));
    let (is_close, kind_str) = rest
        .strip_suffix("終わり")
        .map_or((false, rest), |k| (true, k));
    let kind = bouten_kind_from_suffix(kind_str)?;
    Some((kind, position, is_close))
}

fn parse_small_script_range_body(body: &str) -> Option<(BoutenPosition, bool)> {
    let (is_close, core) = body
        .strip_suffix("終わり")
        .map_or((false, body), |c| (true, c));
    let side = match core {
        "行右小書き" => BoutenPosition::Right,
        "行左小書き" => BoutenPosition::Left,
        _ => return None,
    };
    Some((side, is_close))
}

fn parse_caption_body(body: &str) -> Option<(bool, bool)> {
    Some(match body {
        "キャプション" => (false, false),
        "キャプション終わり" => (false, true),
        "ここからキャプション" => (true, false),
        "ここでキャプション終わり" => (true, true),
        _ => return None,
    })
}

/// ADR-0026 preserves the corpus spelling `、ゴシック体` for render-only interpretation.
fn parse_line_font_size(body: &str) -> Option<(AbsoluteSize, bool)> {
    let (size, rest) = if let Some(r) = body.strip_prefix("特大文字") {
        (AbsoluteSize::ExtraLarge, r)
    } else if let Some(r) = body.strip_prefix("大文字") {
        (AbsoluteSize::Large, r)
    } else if let Some(r) = body.strip_prefix("中文字") {
        (AbsoluteSize::Medium, r)
    } else {
        let r = body.strip_prefix("小文字")?;
        (AbsoluteSize::Small, r)
    };
    let bold = match rest {
        "" => false,
        "、太字" => true,
        _ => return None,
    };
    Some((size, bold))
}

#[derive(Clone, Copy)]
pub(super) enum EmphasisWeight {
    Bold,
    Gothic,
    Italic,
}

pub(super) fn parse_emphasis_body(body: &str) -> Option<(EmphasisWeight, bool, bool)> {
    use EmphasisWeight::{Bold, Gothic, Italic};
    Some(match body {
        "太字" => (Bold, false, false),
        "太字終わり" => (Bold, false, true),
        "ここから太字" => (Bold, true, false),
        "ここで太字終わり" => (Bold, true, true),
        "ゴシック体" => (Gothic, false, false),
        "ゴシック体終わり" => (Gothic, false, true),
        "ここからゴシック体" => (Gothic, true, false),
        "ここでゴシック体終わり" => (Gothic, true, true),
        "斜体" => (Italic, false, false),
        "斜体終わり" => (Italic, false, true),
        "ここから斜体" => (Italic, true, false),
        "ここで斜体終わり" => (Italic, true, true),
        _ => return None,
    })
}

pub(super) fn parse_decimal_u8_prefix(s: &str) -> Option<(u8, &str)> {
    let mut value: u32 = 0;
    let mut consumed = 0;
    for (idx, ch) in s.char_indices() {
        let digit = match ch {
            '0'..='9' => Some(u32::from(ch) - u32::from('0')),
            '０'..='９' => Some(u32::from(ch) - u32::from('０')),
            _ => None,
        };
        let Some(d) = digit else { break };
        value = value.saturating_mul(DECIMAL_RADIX).saturating_add(d);
        consumed = idx + ch.len_utf8();
    }
    if consumed == 0 {
        return None;
    }
    let value_u8 = u8::try_from(value).ok()?;
    Some((value_u8, &s[consumed..]))
}

#[cfg(test)]
mod both_margin_tests {
    use super::*;

    #[test]
    fn corpus_both_margin_spellings_all_resolve() {
        for (amount, tail, want) in [
            (21, "字下げ、地より2字あきで", (21u8, 2u8)),
            (31, "字下げ、地より2字上げで", (31, 2)),
            (28, "字下げて、地より3字上げて", (28, 3)),
            (20, "字下げて、地より1字あきで", (20, 1)),
            (32, "字下げて地より3字上げで", (32, 3)),
        ] {
            assert_eq!(
                parse_both_margin_tail(amount, tail),
                Some(LineFormat::Indent {
                    amount: want.0,
                    end_offset: Some(want.1),
                }),
                "both-margin tail {tail:?} must resolve",
            );
        }
    }

    #[test]
    fn non_both_margin_tails_decline() {
        assert_eq!(parse_both_margin_tail(2, "字下げ"), None);
        assert_eq!(parse_both_margin_tail(2, "字下げ、地より字上げで"), None);
        assert_eq!(parse_both_margin_tail(2, "字下げ、地より2字上げでX"), None);
    }
}

#[cfg(test)]
mod tests {
    //! Verifies body classification and individual keyword parsers.

    use core::ptr;

    use super::*;
    use crate::syntax::ast::Node;

    #[test]
    fn prewarm_initializes_the_body_dispatcher() {
        assert!(
            ptr::eq(prewarm(), body_dispatcher()),
            "prewarm must return the initialized dispatcher"
        );
    }

    fn recognized(body: &str) -> bool {
        let mut alloc = Allocator::new();
        classify_annotation_body(body, &mut alloc).is_some()
    }

    fn line_node(body: &str) -> Option<Node> {
        let mut alloc = Allocator::new();
        match classify_annotation_body(body, &mut alloc) {
            Some((EmitKind::Aozora(node), _)) => Some(node),
            _ => None,
        }
    }

    fn block_open(body: &str) -> Option<RegionFormat> {
        let mut alloc = Allocator::new();
        match classify_annotation_body(body, &mut alloc) {
            Some((EmitKind::BlockOpen(region), _)) => Some(region),
            _ => None,
        }
    }

    fn block_close(body: &str) -> Option<RegionClose> {
        let mut alloc = Allocator::new();
        match classify_annotation_body(body, &mut alloc) {
            Some((EmitKind::BlockClose(close), _)) => Some(close),
            _ => None,
        }
    }

    fn sashie(body: &str) -> Option<(String, Option<String>, Option<String>, bool)> {
        let mut alloc = Allocator::new();
        let emit = classify_sashie_body(body, &mut alloc)?;
        let EmitKind::Aozora(node) = emit else {
            panic!("sashie must classify to an Aozora leaf")
        };
        let Node::Illustration(ill) = node else {
            panic!("sashie must classify to an Illustration, got {node:?}");
        };
        let store = alloc.store();
        Some((
            store.resolve_str(ill.file).to_owned(),
            ill.number.map(|s| store.resolve_str(s).to_owned()),
            ill.dimensions.map(|s| store.resolve_str(s).to_owned()),
            ill.caption.is_some(),
        ))
    }

    fn general(body: &str) -> Option<(String, String, Option<String>)> {
        let mut alloc = Allocator::new();
        let emit = classify_general_image_body(body, &mut alloc)?;
        let EmitKind::Aozora(node) = emit else {
            panic!("general image must classify to an Aozora leaf")
        };
        let Node::Illustration(ill) = node else {
            panic!("general image must classify to an Illustration, got {node:?}");
        };
        let store = alloc.store();
        Some((
            store
                .resolve_str(
                    ill.description
                        .expect("general image carries a description"),
                )
                .to_owned(),
            store.resolve_str(ill.file).to_owned(),
            ill.dimensions.map(|s| store.resolve_str(s).to_owned()),
        ))
    }

    fn emphasis(body: &str) -> Option<(u8, bool, bool)> {
        parse_emphasis_body(body).map(|(weight, padded, is_close)| {
            let tag = match weight {
                EmphasisWeight::Bold => 0,
                EmphasisWeight::Gothic => 1,
                EmphasisWeight::Italic => 2,
            };
            (tag, padded, is_close)
        })
    }

    fn empty_indent_block() -> IndentBlock {
        IndentBlock {
            amount: 1,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        }
    }

    #[test]
    fn editorial_note_sic_family() {
        assert_eq!(editorial_note_kind("ママ"), Some(DirectiveKind::Sic));
        assert_eq!(editorial_note_kind("犬はママ"), Some(DirectiveKind::Sic));
        assert_eq!(editorial_note_kind("底本のまま"), Some(DirectiveKind::Sic));
    }

    #[test]
    fn editorial_note_base_text_variant() {
        assert_eq!(
            editorial_note_kind("「犬」は底本では「猫」"),
            Some(DirectiveKind::BaseTextVariant)
        );
        assert_eq!(
            editorial_note_kind("初出では脱落"),
            Some(DirectiveKind::BaseTextVariant)
        );
    }

    #[test]
    fn editorial_note_pairs_and_editor() {
        assert_eq!(
            editorial_note_kind("入力者注(6)"),
            Some(DirectiveKind::EditorNote)
        );
        assert_eq!(
            editorial_note_kind("「犬」にルビ"),
            Some(DirectiveKind::RubyAttached)
        );
        assert_eq!(
            editorial_note_kind("ルビは「犬」にかかる"),
            Some(DirectiveKind::RubyRetarget)
        );
        assert_eq!(
            editorial_note_kind("左にルビ付き"),
            Some(DirectiveKind::RubyPairOpen)
        );
        assert_eq!(
            editorial_note_kind("左に「かい」のルビ付き終わり"),
            Some(DirectiveKind::RubyPairClose)
        );
        assert_eq!(
            editorial_note_kind("注記付き"),
            Some(DirectiveKind::MarginNotePairOpen)
        );
        assert_eq!(
            editorial_note_kind("左に注記付き"),
            Some(DirectiveKind::MarginNotePairOpen)
        );
        assert_eq!(
            editorial_note_kind("「かい」の注記付き終わり"),
            Some(DirectiveKind::MarginNotePairClose)
        );
        assert_eq!(editorial_note_kind("ただの文"), None);
    }

    #[test]
    fn ruby_attached_body_boundary() {
        assert!(is_ruby_attached_body("「犬」にルビ"));
        assert!(!is_ruby_attached_body("「」にルビ"));
        assert!(!is_ruby_attached_body("にルビ"));
    }

    #[test]
    fn ruby_retarget_body_boundary() {
        assert!(is_ruby_retarget_body("ルビは「犬」にかかる"));
        assert!(!is_ruby_retarget_body("ルビは「」にかかる"));
        assert!(!is_ruby_retarget_body("犬にかかる"));
    }

    #[test]
    fn ruby_pair_close_body_boundary() {
        assert!(is_ruby_pair_close_body("左に「かい」のルビ付き終わり"));
        assert!(!is_ruby_pair_close_body("左に「」のルビ付き終わり"));
        assert!(!is_ruby_pair_close_body("「かい」のルビ付き終わり"));
    }

    #[test]
    fn margin_note_pair_close_body_boundary() {
        assert!(is_margin_note_pair_close_body("「かい」の注記付き終わり"));
        assert!(is_margin_note_pair_close_body(
            "左に「かい」の注記付き終わり"
        ));
        assert!(!is_margin_note_pair_close_body("「」の注記付き終わり"));
        assert!(!is_margin_note_pair_close_body("かいの注記付き終わり"));
    }

    #[test]
    fn editor_note_body_boundary() {
        assert!(is_editor_note_body("入力者注(6)"));
        assert!(is_editor_note_body("入力者注(12)"));
        assert!(!is_editor_note_body("入力者注()"));
        assert!(!is_editor_note_body("入力者注(a)"));
        assert!(!is_editor_note_body("他の注(6)"));
    }

    #[test]
    fn center_marker_page_flag() {
        assert_eq!(
            line_node("ページの左右中央"),
            Some(Node::Line(LineFormat::Center { page: true }))
        );
        assert_eq!(
            line_node("中央揃え"),
            Some(Node::Line(LineFormat::Center { page: false }))
        );
    }

    #[test]
    fn indent_block_end_tail() {
        assert_eq!(
            block_close("ここで字下げ終わり"),
            Some(RegionClose::Indent { kumi_width: None })
        );
        assert!(!recognized("ここで字下げ終わり、活字"));
    }

    #[test]
    fn indent_kumi_block_end_width() {
        assert_eq!(
            block_close("ここで字下げ、20字組み終わり"),
            Some(RegionClose::Indent {
                kumi_width: Some(LineWidth(NonZeroU8::new(20).unwrap())),
            })
        );
    }

    #[test]
    fn font_size_block_end_direction() {
        assert_eq!(
            block_close("ここで大きな文字終わり"),
            Some(RegionClose::FontSize { larger: true })
        );
        assert_eq!(
            block_close("ここで小さな文字終わり"),
            Some(RegionClose::FontSize { larger: false })
        );
    }

    #[test]
    fn columns_block_end_tail() {
        assert_eq!(block_close("ここで2段組終わり"), Some(RegionClose::Columns));
        assert_eq!(
            block_close("ここで2段組み終わり"),
            Some(RegionClose::Columns)
        );
    }

    #[test]
    fn align_end_param_prefix_offset() {
        assert_eq!(
            line_node("地から3字上げ"),
            Some(Node::Line(LineFormat::AlignEnd { offset: 3 }))
        );
        assert!(!recognized("地から0字上げ"));
    }

    #[test]
    fn top_indent_prefix() {
        assert_eq!(
            line_node("天から3字下げ"),
            Some(Node::Line(LineFormat::Indent {
                amount: 3,
                end_offset: None,
            }))
        );
        assert!(!recognized("天から0字下げ"));
    }

    #[test]
    fn kaigyou_tentsuki_prefix() {
        assert_eq!(
            block_open("改行天付き、折り返して3字下げ"),
            Some(RegionFormat::Indent(IndentBlock {
                amount: 0,
                wrap: Some(3),
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            }))
        );
    }

    #[test]
    fn indent_block_param_prefix_forms() {
        assert_eq!(
            block_open("ここから改行天付き、折り返して3字下げ"),
            Some(RegionFormat::Indent(IndentBlock {
                amount: 0,
                wrap: Some(3),
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            }))
        );
        assert_eq!(
            block_open("ここから20字詰め"),
            Some(RegionFormat::LineWidth(LineWidth(
                NonZeroU8::new(20).unwrap()
            )))
        );
        assert_eq!(
            block_open("ここから3段組"),
            Some(RegionFormat::Columns(ColumnCount(
                NonZeroU8::new(3).unwrap()
            )))
        );
        assert_eq!(
            block_open("ここから3段組み"),
            Some(RegionFormat::Columns(ColumnCount(
                NonZeroU8::new(3).unwrap()
            )))
        );
    }

    #[test]
    fn indent_param_prefix_line_form() {
        assert_eq!(
            line_node("3字下げ"),
            Some(Node::Line(LineFormat::Indent {
                amount: 3,
                end_offset: None,
            }))
        );
        assert!(!recognized("0字下げ"));
    }

    #[test]
    fn okurigana_body_length_and_class_boundary() {
        assert!(is_okurigana_body("（あ）"));
        assert!(is_okurigana_body("（あいうえおか）"));
        assert!(!is_okurigana_body("（Ａ）"));
        assert!(!is_okurigana_body("あ"));
    }

    #[test]
    fn okurigana_char_class() {
        assert!(is_okurigana_char('あ'));
        assert!(is_okurigana_char('ア'));
        assert!(is_okurigana_char('漢'));
        assert!(!is_okurigana_char('A'));
        assert!(!is_okurigana_char('Ａ'));
    }

    #[test]
    fn sashie_plain_bare() {
        assert_eq!(
            sashie("挿絵（fig.png）入る"),
            Some(("fig.png".to_owned(), None, None, false))
        );
    }

    #[test]
    fn sashie_numbered() {
        assert_eq!(
            sashie("挿絵1（fig.png）入る"),
            Some(("fig.png".to_owned(), Some("1".to_owned()), None, false))
        );
        assert_eq!(
            sashie("挿絵１（fig.png）入る"),
            Some(("fig.png".to_owned(), Some("１".to_owned()), None, false))
        );
    }

    #[test]
    fn sashie_dimensions_split() {
        assert_eq!(
            sashie("挿絵（fig.png、横100×縦200）入る"),
            Some((
                "fig.png".to_owned(),
                None,
                Some("横100×縦200".to_owned()),
                false,
            ))
        );
        assert_eq!(
            sashie("挿絵（fig.png、）入る"),
            Some(("fig.png、".to_owned(), None, None, false))
        );
    }

    #[test]
    fn general_image_plain() {
        assert_eq!(
            general("地図（map.png）入る"),
            Some(("地図".to_owned(), "map.png".to_owned(), None))
        );
    }

    #[test]
    fn general_image_dimensions_split() {
        assert_eq!(
            general("地図（map.png、横10×縦20）入る"),
            Some((
                "地図".to_owned(),
                "map.png".to_owned(),
                Some("横10×縦20".to_owned()),
            ))
        );
        assert_eq!(
            general("地図（map.png、）入る"),
            Some(("地図".to_owned(), "map.png、".to_owned(), None))
        );
    }

    #[test]
    fn parse_heading_keyword_levels_and_styles() {
        assert_eq!(
            parse_heading_keyword("大見出し"),
            Some((HeadingStyle::Standard, HeadingKind::Large))
        );
        assert_eq!(
            parse_heading_keyword("中見出し"),
            Some((HeadingStyle::Standard, HeadingKind::Medium))
        );
        assert_eq!(
            parse_heading_keyword("小見出し"),
            Some((HeadingStyle::Standard, HeadingKind::Small))
        );
        assert_eq!(
            parse_heading_keyword("同行中見出し"),
            Some((HeadingStyle::SameLine, HeadingKind::Medium))
        );
        assert_eq!(
            parse_heading_keyword("窓小見出し"),
            Some((HeadingStyle::Window, HeadingKind::Small))
        );
        assert_eq!(parse_heading_keyword("副見出し"), None);
    }

    #[test]
    fn strip_heading_style_prefixes() {
        assert_eq!(
            strip_heading_style("同行大見出し"),
            (HeadingStyle::SameLine, "大見出し")
        );
        assert_eq!(
            strip_heading_style("窓大見出し"),
            (HeadingStyle::Window, "大見出し")
        );
        assert_eq!(
            strip_heading_style("大見出し"),
            (HeadingStyle::Standard, "大見出し")
        );
    }

    #[test]
    fn parse_heading_close_levels() {
        assert_eq!(
            parse_heading_close_level("大見出し"),
            Some((HeadingStyle::Standard, Some(HeadingKind::Large)))
        );
        assert_eq!(
            parse_heading_close_level("中見出し"),
            Some((HeadingStyle::Standard, Some(HeadingKind::Medium)))
        );
        assert_eq!(
            parse_heading_close_level("小見出し"),
            Some((HeadingStyle::Standard, Some(HeadingKind::Small)))
        );
        assert_eq!(
            parse_heading_close_level("見出し"),
            Some((HeadingStyle::Standard, None))
        );
        assert_eq!(parse_heading_close_level("窓見出し"), None);
    }

    #[test]
    fn font_size_block_open_steps_sign_and_zero() {
        assert_eq!(font_size_block_open_steps("段階大きな文字", 3), Some(3));
        assert_eq!(font_size_block_open_steps("段階小さな文字", 3), Some(-3));
        assert_eq!(font_size_block_open_steps("段階大きな文字", 0), None);
        assert_eq!(font_size_block_open_steps("段階中くらいな文字", 3), None);
    }

    #[test]
    fn resolve_indent_wrap_clause() {
        let mut block = empty_indent_block();
        assert_eq!(
            resolve_indent_segment("折り返して3字下げ", &mut block),
            Some(())
        );
        assert_eq!(block.wrap, Some(3));
        let mut fresh = empty_indent_block();
        assert_eq!(
            resolve_indent_segment("折り返して3字あき", &mut fresh),
            None
        );
        assert_eq!(fresh.wrap, None);
    }

    #[test]
    fn resolve_indent_layout_clause() {
        let mut block = empty_indent_block();
        assert_eq!(resolve_indent_segment("5字詰め", &mut block), Some(()));
        assert_eq!(
            block.layout,
            IndentLayout::LineWidth(LineWidth(NonZeroU8::new(5).unwrap()))
        );
    }

    #[test]
    fn resolve_indent_style_clauses() {
        let mut gothic = empty_indent_block();
        assert_eq!(resolve_indent_segment("ゴシック体", &mut gothic), Some(()));
        assert!(gothic.styles.gothic);

        let mut horizontal = empty_indent_block();
        assert_eq!(resolve_indent_segment("横書き", &mut horizontal), Some(()));
        assert!(horizontal.styles.horizontal);

        let mut framed = empty_indent_block();
        assert_eq!(resolve_indent_segment("罫囲み", &mut framed), Some(()));
        assert!(framed.styles.framed);

        let mut font = empty_indent_block();
        assert_eq!(resolve_indent_segment("小さい活字", &mut font), Some(()));
        assert_eq!(
            font.styles.font,
            Some(FontShift(NonZeroI8::new(-1).unwrap()))
        );
    }

    #[test]
    fn resolve_indent_repeated_style_declines() {
        let mut gothic = empty_indent_block();
        gothic.styles.gothic = true;
        assert_eq!(resolve_indent_segment("ゴシック体", &mut gothic), None);

        let mut horizontal = empty_indent_block();
        horizontal.styles.horizontal = true;
        assert_eq!(resolve_indent_segment("横書き", &mut horizontal), None);

        let mut framed = empty_indent_block();
        framed.styles.framed = true;
        assert_eq!(resolve_indent_segment("罫囲み", &mut framed), None);

        let mut font = empty_indent_block();
        font.styles.font = Some(FontShift(NonZeroI8::new(1).unwrap()));
        assert_eq!(resolve_indent_segment("小さい活字", &mut font), None);
    }

    #[test]
    fn parse_indent_line_layout_forms() {
        assert_eq!(
            parse_indent_line_layout("5字詰め"),
            Some(IndentLayout::LineWidth(LineWidth(
                NonZeroU8::new(5).unwrap()
            )))
        );
        assert_eq!(
            parse_indent_line_layout("3行20字組み"),
            Some(IndentLayout::Kumi(Kumi {
                lines: NonZeroU8::new(3).unwrap(),
                width: NonZeroU8::new(20).unwrap(),
            }))
        );
        assert_eq!(parse_indent_line_layout("字詰め"), None);
    }

    #[test]
    fn parse_small_script_range_forms() {
        assert_eq!(
            parse_small_script_range_body("行右小書き"),
            Some((BoutenPosition::Right, false))
        );
        assert_eq!(
            parse_small_script_range_body("行右小書き終わり"),
            Some((BoutenPosition::Right, true))
        );
        assert_eq!(
            parse_small_script_range_body("行左小書き"),
            Some((BoutenPosition::Left, false))
        );
        assert_eq!(
            parse_small_script_range_body("行左小書き終わり"),
            Some((BoutenPosition::Left, true))
        );
        assert_eq!(parse_small_script_range_body("行右小書きほげ"), None);
    }

    #[test]
    fn parse_caption_forms() {
        assert_eq!(parse_caption_body("キャプション"), Some((false, false)));
        assert_eq!(
            parse_caption_body("キャプション終わり"),
            Some((false, true))
        );
        assert_eq!(
            parse_caption_body("ここからキャプション"),
            Some((true, false))
        );
        assert_eq!(
            parse_caption_body("ここでキャプション終わり"),
            Some((true, true))
        );
        assert_eq!(parse_caption_body("ほげ"), None);
    }

    #[test]
    fn parse_line_font_size_forms() {
        assert_eq!(
            parse_line_font_size("大文字"),
            Some((AbsoluteSize::Large, false))
        );
        assert_eq!(
            parse_line_font_size("大文字、太字"),
            Some((AbsoluteSize::Large, true))
        );
        assert_eq!(
            parse_line_font_size("特大文字"),
            Some((AbsoluteSize::ExtraLarge, false))
        );
        assert_eq!(
            parse_line_font_size("小文字、太字"),
            Some((AbsoluteSize::Small, true))
        );
        assert_eq!(parse_line_font_size("大文字げ"), None);
    }

    #[test]
    fn parse_emphasis_all_arms() {
        assert_eq!(emphasis("太字"), Some((0, false, false)));
        assert_eq!(emphasis("太字終わり"), Some((0, false, true)));
        assert_eq!(emphasis("ここから太字"), Some((0, true, false)));
        assert_eq!(emphasis("ここで太字終わり"), Some((0, true, true)));
        assert_eq!(emphasis("ゴシック体"), Some((1, false, false)));
        assert_eq!(emphasis("ゴシック体終わり"), Some((1, false, true)));
        assert_eq!(emphasis("ここからゴシック体"), Some((1, true, false)));
        assert_eq!(emphasis("ここでゴシック体終わり"), Some((1, true, true)));
        assert_eq!(emphasis("斜体"), Some((2, false, false)));
        assert_eq!(emphasis("斜体終わり"), Some((2, false, true)));
        assert_eq!(emphasis("ここから斜体"), Some((2, true, false)));
        assert_eq!(emphasis("ここで斜体終わり"), Some((2, true, true)));
        assert_eq!(emphasis("ほげ"), None);
    }

    #[test]
    fn parse_decimal_u8_prefix_fullwidth_digit_value() {
        assert_eq!(parse_decimal_u8_prefix("３４字下げ"), Some((34, "字下げ")));
        assert_eq!(parse_decimal_u8_prefix("34字下げ"), Some((34, "字下げ")));
        assert_eq!(parse_decimal_u8_prefix("字下げ"), None);
        assert_eq!(parse_decimal_u8_prefix("300字下げ"), None);
    }
}
