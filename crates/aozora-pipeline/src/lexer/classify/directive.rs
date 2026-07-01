//! Body-keyword directive classifier.
//!
//! The `［＃<keyword>］` body dispatcher: the `BODY_PATTERNS` table and
//! its Aho-Corasick DFA, `classify_annotation_body`, and the per-family
//! body parsers. Operates purely on the trimmed body string (no event
//! context); the forward-reference recognisers that need event context
//! live in the parent module. Extracted verbatim from the classify-stage
//! classifier.

#[cfg(feature = "classify-instrument")]
use super::super::instrumentation::{Subsystem, SubsystemGuard};

use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, Anchored, Input, MatchKind, StartKind};
use core::num::{NonZeroI8, NonZeroU8};

use aozora_syntax::alloc_owned::OwnedAllocator;
use aozora_syntax::owned::DirectiveOwned;
use aozora_syntax::{
    AbsoluteSize, BOUTEN_KINDS, BlockStyles, BoutenKind, BoutenPosition, ColumnCount,
    DirectiveKind, EnclosureKind, FontShift, HeadingKind, HeadingStyle, IndentBlock, IndentLayout,
    Kumi, LineFormat, LineWidth, RegionClose, RegionFormat, SectionKind,
};

use super::EmitKind;

/// One row of [`BODY_PATTERNS`]: the byte sequence the DFA matches at
/// `body[0..match_end]`, and the family that decides what to emit.
#[derive(Clone, Copy)]
struct BodyPattern {
    needle: &'static str,
    family: BodyFamily,
}

/// Outcome category for an anchored AC match against the annotation
/// body. Each variant carries enough information to either emit a
/// constant `EmitKind` directly (when the family is exact-match) or to
/// dispatch to a small per-family parser for the body remainder.
#[derive(Clone, Copy)]
enum BodyFamily {
    // === Exact-match (body must equal needle) ===
    PageBreak,
    BodyEnd,     // 本文終わり
    ForcedBreak, // 改行
    SectionKaicho,
    SectionKaidan,
    SectionKaimihiraki,
    AlignEnd0,           // 地付き
    CenterMarker,        // ページの左右中央 / 中央揃え
    LineBold,            // この行はゴシック体
    KeigakomiOpen,       // 罫囲み
    KeigakomiClose,      // 罫囲み終わり
    IndentBlock1,        // ここから字下げ → Indent { amount: 1 }
    AlignEndBlock0,      // ここから地付き → AlignEnd { offset: 0 }
    IndentBlockEnd,      // ここで字下げ終わり
    AlignEndBlockEnd,    // ここで地付き終わり
    LineWidthBlockEnd,   // ここで字詰め終わり
    TableBlockOpen,      // ここから表
    TableBlockEnd,       // ここで表終わり
    HorizontalBlockOpen, // ここから横組み
    HorizontalBlockEnd,  // ここで横組み終わり
    FontSizeBlockEnd,    // ここで大きな/小さな文字終わり
    ColumnsBlockEnd,     // ここで段組(み)終わり
    WarichuOpen,         // 割り注
    WarichuClose,        // 割り注終わり
    KaeritenSingle,      // body must equal one of 12 single-char marks
    KaeritenCompound,    // body must equal one of 6 compound marks

    // === Prefix-with-parameter (parse body[match_end..]) ===
    AlignEndParamPrefix,      // 地から → 地から{N}字上げ
    SashiePrefix,             // 挿絵（ → 挿絵（X）入る
    IndentBlockParamPrefix,   // ここから → ここから{N}字下げ
    AlignEndBlockParamPrefix, // ここから地から → ここから地から{N}字上げ
    IndentKumiBlockEnd,       // ここで字下げ、 → ここで字下げ、{W}字組み終わり (#78)
    OkuriganaPrefix,          // （ → kaeriten okurigana （X）

    // === Body-equals-pattern then parse from body[0] ===
    IndentParamPrefix, // {digit} → {N}字下げ (re-parse from body[0])

    /// 傍点 / 傍線 range form (`傍点` / `白丸傍点` / `二重傍線` / `左に傍線`
    /// …, with optional `終わり` close suffix). The needle matches the
    /// variant (or the `左に` prefix); `parse_bouten_range_body` reads the
    /// full body for the kind, the `左に` position, and the `終わり` close.
    BoutenRange,

    /// 太字 / 斜体 range / block form (`太字` / `斜体` inline,
    /// `ここから太字` / `ここで斜体終わり` block, with `終わり` close). The
    /// needle anchors the body; `parse_emphasis_body` reads the full body
    /// for the kind, the block vs inline form, and open vs close.
    Emphasis,

    /// 小書き range form (`行右小書き` / `行左小書き`, with optional `終わり`
    /// close). The bare-range sibling of the forward `「X」は行右小書き`
    /// emphasis leaf; `parse_small_script_range_body` reads the full body
    /// for the 右/左 side and open vs close.
    SmallScriptRange,

    /// キャプション range / block (`キャプション` / `キャプション終わり` inline,
    /// `ここからキャプション` / `ここでキャプション終わり` block).
    /// `parse_caption_body` reads the full body for the block-vs-inline form
    /// and open vs close.
    CaptionRange,

    /// 縦中横 paired range (`縦中横` open / `縦中横終わり` close). A corpus
    /// convention (not in the official 注記一覧, which defines only the
    /// forward-reference `「X」は縦中横` leaf), kept as a tolerant extension.
    /// `parse_tcy_range_body` reads the full body for open vs close.
    CombineUprightRange,

    /// `ここから割り注` — block 割り注 opener (the multi-line region form;
    /// the inline `［＃割り注］` is [`Self::WarichuOpen`]). → `Container(Warichu)`.
    WarichuBlockOpen,
    /// `ここで割り注終わり` — block 割り注 closer.
    WarichuBlockEnd,
    /// `天から` → `天から{N}字下げ` — a single-line indent measured from the
    /// top margin; identical to a plain `{N}字下げ`, so it emits an
    /// `Indent` leaf.
    TopIndentPrefix,
    /// `改行天付き` → `改行天付き、折り返して{N}字下げ` — the ここから-less
    /// bare sibling of the top-flush hanging indent (amount 0 + wrap N).
    KaigyouTentsukiPrefix,

    /// Absolute font-size line directive (`大文字` … `特大文字、太字`). The
    /// needle anchors on the size keyword; `parse_line_font_size` re-reads the
    /// full body for the size and an optional `、太字` / `、ゴシック体` compound.
    LineFontSize,
}

/// How a [`BodyFamily`] consumes its DFA match: an `Exact` family must
/// equal the whole body, a `Prefix` family parses `body[match_end..]`,
/// and a `Reparse` family re-reads the full body from `body[0]`. Derived
/// 1:1 from the family so the exact-vs-not contract lives in one place
/// instead of being split across the per-arm `if exact` guards and a
/// parallel catch-all `None`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Exact,
    Prefix,
    Reparse,
}

/// The [`MatchMode`] of a [`BodyFamily`] (see [`MatchMode`]).
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
        | BodyFamily::LineBold
        | BodyFamily::KeigakomiOpen
        | BodyFamily::KeigakomiClose
        | BodyFamily::WarichuBlockOpen
        | BodyFamily::WarichuBlockEnd
        | BodyFamily::IndentBlock1
        | BodyFamily::AlignEndBlock0
        | BodyFamily::AlignEndBlockEnd
        | BodyFamily::LineWidthBlockEnd
        | BodyFamily::TableBlockOpen
        | BodyFamily::TableBlockEnd
        | BodyFamily::HorizontalBlockOpen
        | BodyFamily::HorizontalBlockEnd
        | BodyFamily::FontSizeBlockEnd
        | BodyFamily::ColumnsBlockEnd
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
        | BodyFamily::OkuriganaPrefix
        | BodyFamily::TopIndentPrefix
        | BodyFamily::KaigyouTentsukiPrefix => MatchMode::Prefix,
        BodyFamily::IndentParamPrefix
        | BodyFamily::BoutenRange
        | BodyFamily::Emphasis
        | BodyFamily::SmallScriptRange
        | BodyFamily::CaptionRange
        | BodyFamily::LineFontSize
        | BodyFamily::CombineUprightRange => MatchMode::Reparse,
    }
}

/// Static pattern table. Order is irrelevant for behavior because the
/// DFA is built with [`MatchKind::LeftmostLongest`]: the longer needle
/// always wins (so `罫囲み終わり` beats `罫囲み`, `ここから字下げ` beats
/// `ここから`, `一レ` beats `一`, etc.). Keeping families together for
/// readability instead of sorting by length.
static BODY_PATTERNS: &[BodyPattern] = &[
    // Block container with full-keyword bodies.
    BodyPattern {
        needle: "ここから字下げ",
        family: BodyFamily::IndentBlock1,
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
    // The 字組み compound closer carries the width (`ここで字下げ、20字組み終わり`,
    // #78). Distinct from the generic `ここで字下げ終わり` above — the char after
    // `ここで字下げ` is `、` vs `終`, so the two needles never overlap.
    BodyPattern {
        needle: "ここで字下げ、",
        family: BodyFamily::IndentKumiBlockEnd,
    },
    BodyPattern {
        needle: "ここで地付き終わり",
        family: BodyFamily::AlignEndBlockEnd,
    },
    // The 字上げ block (［＃ここから地から N 字上げ］) is closed by either
    // ［＃ここで字上げ終わり］ or ［＃ここで地付き終わり］ — both end the same
    // AlignEnd container. The open-side offset is authoritative when
    // pairing, so this closer reuses AlignEndBlockEnd.
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
    // Bare-range font-size close (ここ-less): ［＃大きな/小さな文字終わり］,
    // the sibling of ここで…終わり. Reuses FontSizeBlockEnd; the open side
    // (［＃{N}段階…文字］) routes through IndentParamPrefix (leading digit).
    // LeftmostLongest keeps ここで… winning over the bare needle.
    BodyPattern {
        needle: "大きな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    BodyPattern {
        needle: "小さな文字終わり",
        family: BodyFamily::FontSizeBlockEnd,
    },
    // Bare-range horizontal (ここ-less): ［＃横組み］ … ［＃横組み終わり］,
    // the sibling of ここから横組み / ここで横組み終わり. Same Horizontal
    // container; LeftmostLongest keeps 横組み終わり winning over 横組み, and
    // the exact-match guard rejects compounds like 横組みで、… (→ Unknown).
    BodyPattern {
        needle: "横組み",
        family: BodyFamily::HorizontalBlockOpen,
    },
    BodyPattern {
        needle: "横組み終わり",
        family: BodyFamily::HorizontalBlockEnd,
    },
    // 小書き range: ［＃行右小書き］ … ［＃行右小書き終わり］ (and 行左).
    // LeftmostLongest keeps 行右小書き終わり winning over 行右小書き.
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
    // キャプション range / block. LeftmostLongest keeps キャプション終わり
    // over キャプション, and ここからキャプション over ここから.
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
    // 縦中横 paired range. The bare 縦中横 needle anchors both the opener and
    // its 縦中横終わり closer (re-parsed by `parse_tcy_range_body`). The
    // forward-reference `「X」は縦中横` leaf starts with `「`, so it is not
    // claimed here.
    BodyPattern {
        needle: "縦中横",
        family: BodyFamily::CombineUprightRange,
    },
    // Block 罫囲み (ここから form; the bare 罫囲み is also KeigakomiOpen).
    // LeftmostLongest keeps ここから罫囲み over the ここから indent prefix.
    BodyPattern {
        needle: "ここから罫囲み",
        family: BodyFamily::KeigakomiOpen,
    },
    BodyPattern {
        needle: "ここで罫囲み終わり",
        family: BodyFamily::KeigakomiClose,
    },
    // 表罫囲み (table-rule frame) renders identically to 罫囲み in the
    // reference converter, so it folds into the same Rule enclosure (like
    // 枠囲み/枠囲い) and canonicalizes to 罫囲み on serialize.
    BodyPattern {
        needle: "ここから表罫囲み",
        family: BodyFamily::KeigakomiOpen,
    },
    BodyPattern {
        needle: "ここで表罫囲み終わり",
        family: BodyFamily::KeigakomiClose,
    },
    // Block 割り注 (multi-line region; inline ［＃割り注］ stays WarichuOpen).
    BodyPattern {
        needle: "ここから割り注",
        family: BodyFamily::WarichuBlockOpen,
    },
    BodyPattern {
        needle: "ここで割り注終わり",
        family: BodyFamily::WarichuBlockEnd,
    },
    // 天から{N}字下げ (single-line indent from the top) and the bare
    // 改行天付き、折り返して{N}字下げ hanging indent.
    BodyPattern {
        needle: "天から",
        family: BodyFamily::TopIndentPrefix,
    },
    BodyPattern {
        needle: "改行天付き",
        family: BodyFamily::KaigyouTentsukiPrefix,
    },
    BodyPattern {
        needle: "ここで段組終わり",
        family: BodyFamily::ColumnsBlockEnd,
    },
    BodyPattern {
        needle: "ここで段組み終わり",
        family: BodyFamily::ColumnsBlockEnd,
    },
    // Section / page break (exact).
    BodyPattern {
        needle: "改ページ",
        family: BodyFamily::PageBreak,
    },
    // 改頁 — the kanji spelling of 改ページ (annotation/layout_1.html);
    // canonicalises to 改ページ on serialize.
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
    // Structural markers (#78). `改行` is an exact match: a bare `［＃改行］`
    // forced line break. The longer `改行天付き` needle wins under
    // LeftmostLongest for the hanging-indent form, and the Exact mode rejects
    // any `改行X` tail, so only the bare body reaches `ForcedBreak`.
    BodyPattern {
        needle: "本文終わり",
        family: BodyFamily::BodyEnd,
    },
    BodyPattern {
        needle: "改行",
        family: BodyFamily::ForcedBreak,
    },
    // Geographic alignment.
    BodyPattern {
        needle: "地から",
        family: BodyFamily::AlignEndParamPrefix,
    },
    // 地より — the alternate wording of 地から (both "measured from the
    // bottom margin"); `地よりN字上げ` parses identically and canonicalises
    // to 地から on serialize. LeftmostLongest keeps ここから地より winning.
    BodyPattern {
        needle: "地より",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "ここから地より",
        family: BodyFamily::AlignEndBlockParamPrefix,
    },
    // 文末より / 行末より — raised alignment measured from the END of the
    // text (vs 地から = bottom margin). Same zero-width AlignEnd hook.
    // `この行は行末より…` needs its own anchored needle.
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
    // 右寄せ / 地寄せ — wording variants of 地付き (inline-end alignment; in
    // horizontal render inline-end == the right edge). Canonicalize to 地付き.
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
        family: BodyFamily::LineBold,
    },
    // Absolute font-size line directives (`大文字` … `特大文字、太字`). All four
    // size keywords anchor `LineFontSize`; `parse_line_font_size` re-reads the
    // body for the size and the optional `、太字` compound. `特大文字` is the
    // longest, so LeftmostLongest prefers it over `大文字`.
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
    // Other inline / block. Needle is bare 挿絵 (not 挿絵（) so the numbered
    // form 挿絵{N}（…） also reaches classify_sashie_body, which re-validates.
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
    // 傍点 / 傍線 range form openers (`［＃傍点］ … ［＃傍点終わり］`). One
    // needle per emphasis variant `bouten_kind_from_suffix` recognises,
    // plus the `左に` left-side prefix. LeftmostLongest disambiguates
    // overlaps (`二重丸傍点` vs `丸傍点`, `白丸傍点` vs `丸傍点`); the close
    // form (`…終わり`) matches the same variant needle as a prefix and is
    // re-parsed in full by `parse_bouten_range_body`.
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
    // 太字 / 斜体 emphasis. The inline-range openers (`太字` / `斜体`)
    // also anchor their `…終わり` closers (re-parsed in full by
    // `parse_emphasis_body`); the block forms need their own anchors
    // (`ここから太字` beats the generic `ここから` via LeftmostLongest;
    // `ここで太字終わり` has no shorter generic anchor).
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
    // ゴシック体 / ゴチック — corpus spellings of 太字 (bold). The official
    // guide writes 太字（ゴシック） and prescribes 太字 as the keyword
    // (annotation/emphasis.html), so both canonicalise to `太字` on
    // serialize. The bare openers also anchor their `…終わり` closers and
    // the forward-reference `「X」はゴシック体` leaf.
    BodyPattern {
        needle: "ゴシック体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ゴチック",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここからゴシック体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここからゴチック",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここでゴシック体終わり",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここでゴチック終わり",
        family: BodyFamily::Emphasis,
    },
    // Kaeriten okurigana opener (full-width left paren U+FF08).
    BodyPattern {
        needle: "（",
        family: BodyFamily::OkuriganaPrefix,
    },
    // Kaeriten compound marks (6) — must precede the single forms in
    // the table only for documentation; LeftmostLongest does the
    // actual disambiguation (`一レ` 6 bytes > `一` 3 bytes).
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
    // Kaeriten single marks (12).
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
    // {N}字下げ — anchored on each digit (ASCII + full-width).
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

/// Build the annotation-body Aho-Corasick automaton from `BODY_PATTERNS`.
///
/// This DFA build is the bulk of parser boot cost (~150 microseconds, as
/// the `boot` bench measures). It is exposed under `#[doc(hidden)]` so
/// that bench can build it in isolation without making `BODY_PATTERNS`
/// public — the same pattern as `aozora-scan`'s hidden `NaiveScanner`
/// export. The process-lifetime cache lives in `body_dispatcher`;
/// `prewarm` warms it.
#[doc(hidden)]
#[must_use]
pub fn build_body_dispatcher() -> AhoCorasick {
    AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .start_kind(StartKind::Anchored)
        .build(BODY_PATTERNS.iter().map(|p| p.needle))
        .expect("BODY_PATTERNS is a static, non-empty, valid set")
}

/// One-time DFA build, amortised across the entire process lifetime.
/// Lookup cost is a few ns per call so the build pays back in under a
/// thousand annotations.
fn body_dispatcher() -> &'static AhoCorasick {
    static DFA: OnceLock<AhoCorasick> = OnceLock::new();
    DFA.get_or_init(build_body_dispatcher)
}

/// Force the one-time Aho-Corasick DFA build now.
///
/// This is the bulk of parser boot cost. Idempotent — the `OnceLock` is
/// set at most once per process. `body_dispatcher` stays private; this
/// only triggers its init.
pub(crate) fn prewarm() {
    let _ = body_dispatcher();
}

/// Classify an input-editor note body into its [`DirectiveKind`], or
/// `None` if the body is not a recognised editorial note.
///
/// These are the corpus's two dominant editorial families:
/// - `ママ` / `「X」はママ` (and `ルビの「X」はママ`) — *sic*: X is reproduced
///   as it stands in the source. `底本のまま` ("as in the base text") is the
///   same kept-irregularity note. → [`DirectiveKind::Sic`].
/// - `…底本では…` (`「X」は底本では「Y」`, `「X」は底本では脱落`, …) — a
///   source-text divergence note. `…初出では…` ("in the first appearance …")
///   is the same shape against the first publication. →
///   [`DirectiveKind::BaseTextVariant`].
///
/// Called only at the tail of `RecogniseCtx::recognize_annotation`, after
/// every styling recogniser has declined, so a target-bearing form like
/// `「ママ」に傍点` has already been claimed as a Bouten and never reaches
/// here. The note does not restyle its target, so the caller leaves X in
/// the text and consumes only the bracket.
pub(super) fn editorial_note_kind(body: &str) -> Option<DirectiveKind> {
    if body == "ママ" || body.ends_with("はママ") || body == "底本のまま" {
        Some(DirectiveKind::Sic)
    } else if body.contains("底本では") || body.contains("初出では") {
        // Checked before EditorNote so a 底本 correction that happens to cite a
        // numbered note (`底本では…誤記。入力者注(6)`) stays a BaseTextVariant.
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
    } else {
        None
    }
}

/// Whether `body` is exactly a ruby-presence note `「X」にルビ` (whole body, `X`
/// non-empty). A proofreading marker that the run `X` carries a ruby gloss.
fn is_ruby_attached_body(body: &str) -> bool {
    body.strip_prefix("「")
        .and_then(|r| r.strip_suffix("」にルビ"))
        .is_some_and(|x| !x.is_empty())
}

/// Whether `body` is exactly a ruby-binding note `ルビは「X」にかかる` (whole body,
/// `X` non-empty). Records that a nearby ruby applies to the run `X`.
fn is_ruby_retarget_body(body: &str) -> bool {
    body.strip_prefix("ルビは「")
        .and_then(|r| r.strip_suffix("」にかかる"))
        .is_some_and(|x| !x.is_empty())
}

/// Whether `body` is a left-side-ruby span closer `左に「Y」のルビ付き終わり`
/// (whole body, reading `Y` non-empty). The matching opener is the fixed
/// `左にルビ付き` body.
fn is_ruby_pair_close_body(body: &str) -> bool {
    body.strip_prefix("左に「")
        .and_then(|r| r.strip_suffix("」のルビ付き終わり"))
        .is_some_and(|y| !y.is_empty())
}

/// Whether `body` is exactly a numbered input-typist note `入力者注(N)` with an
/// ASCII-paren, ASCII-digit index — the corpus form. A compound note that
/// merely *contains* the phrase is excluded (the whole body must match).
fn is_editor_note_body(body: &str) -> bool {
    let Some(rest) = body.strip_prefix("入力者注(") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(')') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Single-pass classification of `body` (the trimmed bytes between
/// `［＃` and `］`) into an `EmitKind` for body-only annotation
/// families. Returns `None` if the body matches no body-only family;
/// the caller then falls through to forward classifiers and finally
/// the `Directive{Unknown}` catch-all.
#[allow(
    clippy::too_many_lines,
    reason = "single match arm per BodyFamily — splitting would scatter \
              the dispatch logic and obscure the intentional 1:1 mapping"
)]
pub(super) fn classify_annotation_body(
    body: &str,
    alloc: &mut OwnedAllocator,
) -> Option<(EmitKind, Option<DirectiveOwned>)> {
    #[cfg(feature = "classify-instrument")]
    let _phase3_guard = SubsystemGuard::new(Subsystem::BodyDispatcher);
    if body.is_empty() {
        return None;
    }
    // Paired / block headings route through the container machinery as
    // `ContainerKind::Heading`. Tried before the body dispatcher: their
    // keywords overlap the `ここから…` / `…終わり` shapes but always carry a
    // `見出し` keyword, so a non-heading `ここから…` body falls through.
    if let Some((region, is_open)) = parse_heading_directive(body) {
        let emit = if is_open {
            EmitKind::BlockOpen(region)
        } else {
            EmitKind::BlockClose(RegionClose::of(region))
        };
        return Some((emit, None));
    }
    let dfa = body_dispatcher();
    let mat = dfa.find(Input::new(body).anchored(Anchored::Yes))?;
    let pat = BODY_PATTERNS[mat.pattern().as_usize()];
    let match_end = mat.end();
    let exact = match_end == body.len();
    // An exact-match family must consume the whole body; a prefix-only
    // DFA hit (`罫囲みfoo` matches the needle `罫囲み`) makes no claim.
    // Checking the mode once here lets every exact arm below drop its
    // `if exact` guard and replaces the parallel catch-all `None`.
    if body_family_mode(pat.family) == MatchMode::Exact && !exact {
        return None;
    }
    match pat.family {
        // ----- Exact-match families (must consume the entire body) -----
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
            // ページの左右中央 (page centre) vs 中央揃え — a single-line
            // zero-width centring marker.
            let page = body == "ページの左右中央";
            Some((
                EmitKind::Aozora(alloc.line(LineFormat::Center { page })),
                None,
            ))
        }
        BodyFamily::LineBold => Some((EmitKind::Aozora(alloc.line(LineFormat::Bold)), None)),
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
        BodyFamily::AlignEndBlock0 => Some((
            EmitKind::BlockOpen(RegionFormat::AlignEnd { offset: 0 }),
            None,
        )),
        BodyFamily::IndentBlockEnd => {
            // ここで字下げ終わり, optionally with a redundant compound tail
            // `、{style}も終わり` (e.g. `…終わり、小さい活字も終わり`, #78). The
            // open payload is authoritative and the generic 字下げ終わり closes
            // the whole stack, so any `、…も終わり` tail maps to the same generic
            // close (it re-serializes to the canonical `ここで字下げ終わり`). A
            // non-`、` tail is not this family → decline to Unknown.
            let tail = &body[match_end..];
            (tail.is_empty() || (tail.starts_with('、') && tail.ends_with("終わり"))).then_some((
                EmitKind::BlockClose(RegionClose::Indent { kumi_width: None }),
                None,
            ))
        }
        BodyFamily::AlignEndBlockEnd => Some((EmitKind::BlockClose(RegionClose::AlignEnd), None)),
        BodyFamily::LineWidthBlockEnd => {
            // The close marker carries no width; the open-side payload is
            // authoritative when pairing (mirrors the generic 字下げ終わり).
            Some((EmitKind::BlockClose(RegionClose::LineWidth), None))
        }
        BodyFamily::IndentKumiBlockEnd => {
            // ここで字下げ、{W}字組み終わり (#78) — the 字組み compound closer.
            // The close carries its own `W` so the marker round-trips byte-exact
            // (it pairs with the Indent open by family). Tolerate an optional
            // leading `{L}行`. Declines (→ Unknown) on any other shape.
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
            // The close marker carries only the direction (大きな / 小さな);
            // its magnitude is the open's. Matches both ここで…終わり and the
            // bare …終わり sibling, so key on the direction word.
            let larger = !body.contains("小さな");
            Some((EmitKind::BlockClose(RegionClose::FontSize { larger }), None))
        }
        BodyFamily::ColumnsBlockEnd => {
            // Close marker carries no count; the open-side payload is authoritative.
            Some((EmitKind::BlockClose(RegionClose::Columns), None))
        }
        BodyFamily::WarichuOpen => {
            let p = alloc.make_directive("［＃割り注］", DirectiveKind::WarichuOpen);
            let node = alloc.annotation(p);
            // Re-build a payload for the segment-wrap case. The
            // borrowed allocator interns by string content, so the
            // second call hits the dedup table; the owned allocator
            // pays a single `Box<str>` clone, which is cheap relative
            // to the rare nested-Warichu shape this case targets.
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

        // ----- Prefix-with-parameter families -----
        BodyFamily::AlignEndParamPrefix => {
            // body == 地から/文末より/行末より{N}字上げ; remainder = body[match_end..].
            // The verb is the intransitive 字上がり as well as 字上げ, with an
            // optional 揃え suffix (`文末よりN字上げ揃え`).
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
            // body == 天から{N}字下げ — single-line indent from the top
            // margin, identical to a plain {N}字下げ (Indent leaf).
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字下げ" && n >= 1).then(|| {
                (
                    EmitKind::Aozora(alloc.line(LineFormat::Indent { amount: n })),
                    None,
                )
            })
        }
        BodyFamily::KaigyouTentsukiPrefix => {
            // body == 改行天付き、折り返して{N}字下げ — the ここから-less bare
            // top-flush hanging indent (amount 0 + wrap N), closed by the
            // shared 字下げ終わり.
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
            // body == ここから{N}字下げ; remainder = body[match_end..]
            let rest = &body[match_end..];
            // ここから[改行]天付き、折り返して{M}字下げ — top-flush hanging
            // indent: the first line sits at the top margin (天付き = no
            // indent), wrapped continuation lines indent M. Models as the same
            // Indent container with amount 0 + wrap M, so it closes with the
            // shared 字下げ終わり (pairing is by family). Both the `改行天付き`
            // (corpus's most common top form) and the bare `天付き` spellings
            // appear; accept either before the leading-digit parse below.
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
                // ここから{N}字下げ、… compound (#78): the indent opener carries
                // a trailing `、`-separated stack of clauses — `折り返して{M}字下げ`
                // (wrap), `ページの左右中央`/`中央揃え` (center), `{W}字詰め` /
                // `{L}行{W}字組み[で]` (line layout), and the decorative styles
                // `ゴシック体` / `小さい活字` / `横書き` / `罫囲み`. Resolved as a
                // set in canonical-order-independent fashion; the whole compound
                // is declined to a generic Unknown if ANY clause is unrecognised
                // (lossless — e.g. `横組み右揃えで`, `数式`, embedded `「」は返り点`).
                // All forms still close with the shared 字下げ終わり (by family).
                parse_indent_compound(n, after)
                    .map(|block| (EmitKind::BlockOpen(RegionFormat::Indent(block)), None))
            } else if tail == "字詰め" {
                // ここから{N}字詰め — line-width container (字詰め): N
                // full-width characters per line. Shares the `ここから`
                // opener prefix with 字下げ; block-only, closes with
                // `ここで字詰め終わり`. `NonZero` folds the `N >= 1` guard.
                NonZeroU8::new(n).map(|w| {
                    (
                        EmitKind::BlockOpen(RegionFormat::LineWidth(LineWidth(w))),
                        None,
                    )
                })
            } else if tail == "段組" || tail == "段組み" {
                // ここから{N}段組(み) — multi-column container (段組): N
                // columns. Shares the `ここから` prefix; closes with
                // `ここで段組(み)終わり`. `NonZero` folds the `N >= 1` guard.
                NonZeroU8::new(n).map(|c| {
                    (
                        EmitKind::BlockOpen(RegionFormat::Columns(ColumnCount(c))),
                        None,
                    )
                })
            } else {
                // ここから{N}段階大きな/小さな文字 — block font-size shift.
                // Shares the `ここから` prefix; closes with the direction-only
                // `ここで大きな/小さな文字終わり`.
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
            // body == ここから地から{N}字上げ; remainder = body[match_end..]
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字上げ").then_some((
                EmitKind::BlockOpen(RegionFormat::AlignEnd { offset: n }),
                None,
            ))
        }
        BodyFamily::OkuriganaPrefix => {
            // The DFA matched `（` at body[0..3]. Defer to the same
            // parens-recognising helper as the legacy code so the
            // length / character-class invariants stay in one place.
            is_okurigana_body(body).then(|| (EmitKind::Aozora(alloc.kaeriten(body)), None))
        }
        BodyFamily::IndentParamPrefix => {
            // The DFA matched a single digit. Re-parse from body[0]
            // for full multi-digit support.
            let (n, tail) = parse_decimal_u8_prefix(body)?;
            if tail == "字下げ" && n >= 1 {
                Some((
                    EmitKind::Aozora(alloc.line(LineFormat::Indent { amount: n })),
                    None,
                ))
            } else {
                // Bare-range font-size open: ［＃{N}段階大きな/小さな文字］ —
                // the ここから-less sibling of the block opener, closed by the
                // bare ［＃大きな/小さな文字終わり］. Reuses the FontSize
                // region so render / pairing / serialize already apply.
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
            // `傍点` / `白丸傍点` / `二重傍線` / `左に傍線` … with an optional
            // `終わり` close suffix. Re-parse the full body for the variant,
            // the `左に` position, and open vs close.
            let (kind, position, is_close) = parse_bouten_range_body(body)?;
            Some((
                open_or_close(RegionFormat::Bouten { kind, position }, is_close),
                None,
            ))
        }
        BodyFamily::Emphasis => {
            // `太字` / `斜体` / `ここから太字` / `ここで斜体終わり` … —
            // re-parse the full body for the kind, the block vs inline
            // form, and open vs close.
            let (is_italic, padded, is_close) = parse_emphasis_body(body)?;
            let region = if is_italic {
                RegionFormat::Italic { padded }
            } else {
                RegionFormat::Bold { padded }
            };
            Some((open_or_close(region, is_close), None))
        }
        BodyFamily::SmallScriptRange => {
            // `行右小書き` / `行左小書き` with an optional `終わり` close.
            // Re-parse the full body so `行右小書きほげ` (needle prefix but
            // longer body) declines to Directive{Unknown}.
            let (side, is_close) = parse_small_script_range_body(body)?;
            Some((
                open_or_close(RegionFormat::SmallScript(side), is_close),
                None,
            ))
        }
        BodyFamily::CaptionRange => {
            // `キャプション` (inline) / `ここからキャプション` (block) with an
            // optional `終わり` close; re-parse the full body.
            let (padded, is_close) = parse_caption_body(body)?;
            Some((
                open_or_close(RegionFormat::Caption { padded }, is_close),
                None,
            ))
        }

        BodyFamily::CombineUprightRange => {
            // `縦中横` open / `縦中横終わり` close — re-parse the full body so a
            // needle-prefix-but-longer body (`縦中横ほげ`) declines cleanly.
            let is_close = parse_tcy_range_body(body)?;
            Some((open_or_close(RegionFormat::CombineUpright, is_close), None))
        }
    }
}

/// Wrap an open [`RegionFormat`] as the matching [`EmitKind`]: the open marker
/// carries the full payload; the close projects to its [`RegionClose`]
/// discriminant via [`RegionClose::of`] (the open side stays authoritative).
fn open_or_close(region: RegionFormat, is_close: bool) -> EmitKind {
    if is_close {
        EmitKind::BlockClose(RegionClose::of(region))
    } else {
        EmitKind::BlockOpen(region)
    }
}

/// Whether `body` is the okurigana shape `（X）` where X is a short
/// run of Japanese characters.
///
/// The length bound guards against accidentally claiming long
/// parenthesised glosses (which belong to the generic annotation
/// catch-all). 6 characters is the ~99th-percentile okurigana length
/// in Aozora corpora; anything longer is practically always editorial
/// prose rather than an inflection marker.
fn is_okurigana_body(body: &str) -> bool {
    let Some(inner) = body.strip_prefix('（').and_then(|s| s.strip_suffix('）')) else {
        return false;
    };
    // Byte-length prefilter: every accepted okurigana char is a CJK
    // glyph in {hiragana, katakana, half-width katakana, CJK unified}.
    // Hiragana/katakana/CJK are 3 bytes UTF-8; half-width katakana
    // is also 3 bytes (U+FF61..U+FF9F). So a 1..=6 char inner has
    // byte length in `3..=18`. Any inner outside that range cannot
    // satisfy `is_okurigana_char.all` and we skip the char decode.
    if !(3..=18).contains(&inner.len()) {
        return false;
    }
    // Single-pass fusion of `chars().count()` + `chars().all()`:
    // count and class-check in one walk, with early-out at >6 chars
    // or first non-conforming char. Replaces two iterations over
    // the same byte stream.
    let mut count = 0usize;
    for c in inner.chars() {
        count += 1;
        if count > 6 || !is_okurigana_char(c) {
            return false;
        }
    }
    count >= 1
}

/// Character class accepted inside okurigana parens: hiragana,
/// katakana (incl. half-width), CJK unified ideographs. Deliberately
/// narrower than "any non-whitespace" so editorial `（注）` or
/// punctuation-rich glosses fall through to the annotation path.
const fn is_okurigana_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3041}'..='\u{309F}'      // hiragana
        | '\u{30A0}'..='\u{30FF}'    // katakana
        | '\u{FF66}'..='\u{FF9F}'    // half-width katakana
        | '\u{4E00}'..='\u{9FFF}'    // CJK unified
        | '\u{3400}'..='\u{4DBF}'    // CJK ext A
        | '\u{F900}'..='\u{FAFF}'    // CJK compat
    )
}

/// Classify a `［＃挿絵（file）入る］` sashie (illustration insert),
/// optionally bundling a caption: `［＃挿絵（file）「caption」入る］`.
///
/// Called from [`classify_annotation_body`]'s `SashiePrefix` arm —
/// the AC has already verified the `挿絵（` prefix at body[0..9]; this
/// function captures the filename between `（` and `）`, an optional
/// `「caption」` (per <https://www.aozora.gr.jp/annotation/graphics.html>),
/// and confirms the trailing `入る` keyword. The caption is plain content,
/// rendered into `<figcaption>` (§8).
fn classify_sashie_body(body: &str, alloc: &mut OwnedAllocator) -> Option<EmitKind> {
    // `挿絵（file）入る` and the numbered `挿絵{N}（file）入る` (N a run of
    // half/full-width digits before the `（`). A description *before* 挿絵
    // (`女性と犬の挿絵（…）`, `「…」のキャプション付きの挿絵（…）`) is a separate,
    // unhandled form — it does not start with 挿絵, so the needle misses it.
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
    // `）` is a full-width right parenthesis (U+FF09). Find its first
    // occurrence — corpus rarely nests `（）` inside a filename.
    let close_off = rest.find('）')?;
    // The `（…）` body is either a bare `file` or `file、横W×縦H` — split off
    // the optional pixel-size note so `file` stays a clean `<img src>` path
    // and the dimensions render as `width`/`height` (see render_node).
    let inside = &rest[..close_off];
    let (file, dimensions) = match inside.split_once('、') {
        Some((f, dims)) if !f.is_empty() && !dims.is_empty() => (f, Some(dims)),
        _ => (inside, None),
    };
    if file.is_empty() {
        return None;
    }
    let tail = &rest[close_off + '）'.len_utf8()..];
    // After `）` the tail is either the bare `入る` keyword or a bundled
    // `「caption」入る`. Any other shape declines (→ `Directive{Unknown}`).
    let caption = if tail == "入る" {
        None
    } else if let Some(inner) = tail
        .strip_prefix('「')
        .and_then(|t| t.strip_suffix("」入る"))
    {
        if inner.is_empty() {
            return None;
        }
        Some(alloc.content_plain(inner))
    } else {
        return None;
    };
    Some(EmitKind::Aozora(
        alloc.sashie(file, number, dimensions, caption),
    ))
}

/// Classify the *general* image form `［＃<説明>（file［、横W×縦H］）入る］`
/// (図 / 地図 / 口絵 / 表紙 / コンドル博士の図 / 神代文字ア …) per
/// <https://www.aozora.gr.jp/annotation/graphics.html>: the leading text
/// before `（` is the image's alt-description (the guide lists 図 / 地図 /
/// 絵 / 挿絵 / 表 / 写真 as type words but the description is free text),
/// the parenthesised part is `file` (+ optional `、横W×縦H` pixel size),
/// and `入る` closes it.
///
/// The keyword `挿絵` form is claimed earlier by [`classify_sashie_body`]
/// via its anchored needle; this is the fallback for every other
/// description, tried just before the `Directive{Unknown}` catch-all (it
/// has no prefix needle because the description is arbitrary). Returns
/// `None` for any body that is not a complete `<非空>（<file>）入る`.
pub(super) fn classify_general_image_body(
    body: &str,
    alloc: &mut OwnedAllocator,
) -> Option<EmitKind> {
    let middle = body.strip_suffix("入る")?;
    let paren = middle.find('（')?;
    let description = &middle[..paren];
    if description.is_empty() {
        return None;
    }
    let rest = &middle[paren + '（'.len_utf8()..];
    let close_off = rest.find('）')?;
    // Once `入る` is stripped, `）` must be the final byte — a trailing
    // `「caption」` or any other shape is not this form and declines.
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

/// Parse a heading keyword into `(style, kind)`. An optional `同行`
/// (same-line) / `窓` (window) prefix selects the style; the remaining
/// `大 / 中 / 小見出し` selects the level. Shared by the forward-reference
/// hint (`「X」はSTYLEレベル見出し`) and the paired / block container forms
/// ([`parse_heading_directive`]).
///
/// `副見出し` is not a real annotation — it never occurs in the corpus — so
/// it matches nothing and the directive falls through to `Directive{Unknown}`.
/// The 同行 / 窓 styles cross with every level (`同行中見出し`, `窓小見出し`, …).
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

/// Strip an optional 同行 / 窓 style prefix, returning the style and the
/// remaining `大/中/小見出(し)` stem. Shared by the strict open/hint parser
/// and the 送り仮名-tolerant close parser.
fn strip_heading_style(s: &str) -> (HeadingStyle, &str) {
    [
        ("同行", HeadingStyle::SameLine),
        ("窓", HeadingStyle::Window),
    ]
    .into_iter()
    .find_map(|(prefix, style)| s.strip_prefix(prefix).map(|rest| (style, rest)))
    .unwrap_or((HeadingStyle::Standard, s))
}

/// Heading keyword for a **close** marker only. Tolerates the 送り仮名-elided
/// stem (`中見出` for `中見出し`) that appears in the wild solely on the
/// closing `…見出終わり` — the open and forward-hint forms keep requiring the
/// full `見出し` via [`parse_heading_keyword`]. The close still serializes
/// back to the canonical `見出し` keyword, so the round-trip is a fixed point.
fn parse_heading_close_keyword(s: &str) -> Option<(HeadingStyle, HeadingKind)> {
    let (style, rest) = strip_heading_style(s);
    let kind = match rest {
        "大見出し" | "大見出" => HeadingKind::Large,
        "中見出し" | "中見出" => HeadingKind::Medium,
        "小見出し" | "小見出" => HeadingKind::Small,
        _ => return None,
    };
    Some((style, kind))
}

/// Recognise a **paired** (`STYLEレベル見出し` / `…見出し終わり`) or **block**
/// (`ここからSTYLEレベル見出し` / `ここでSTYLEレベル見出し終わり`) heading
/// directive body, returning `(container, is_open)`. These delimit their
/// content and route through the container pairing machinery as
/// [`RegionFormat::Heading`] (the counterpart of the `は`-form leaf heading).
///
/// The forward-reference `「X」は…見出し` hint starts with `「`, so it never
/// matches here; a `ここから…` / `…終わり` body that is not a heading keyword
/// (e.g. `ここから2字下げ`) fails `parse_heading_keyword` and falls through to
/// the body dispatcher.
fn parse_heading_directive(body: &str) -> Option<(RegionFormat, bool)> {
    if let Some(rest) = body.strip_prefix("ここから") {
        let (style, level) = parse_heading_keyword(rest)?;
        return Some((
            RegionFormat::Heading {
                level,
                style,
                padded: true,
            },
            true,
        ));
    }
    if let Some(rest) = body.strip_prefix("ここで") {
        let (style, level) = parse_heading_close_keyword(rest.strip_suffix("終わり")?)?;
        return Some((
            RegionFormat::Heading {
                level,
                style,
                padded: true,
            },
            false,
        ));
    }
    if let Some(inner) = body.strip_suffix("終わり") {
        let (style, level) = parse_heading_close_keyword(inner)?;
        return Some((
            RegionFormat::Heading {
                level,
                style,
                padded: false,
            },
            false,
        ));
    }
    let (style, level) = parse_heading_keyword(body)?;
    Some((
        RegionFormat::Heading {
            level,
            style,
            padded: false,
        },
        true,
    ))
}

/// Signed stage count for a `ここから{N}段階大きな/小さな文字` block opener,
/// where `tail` is the body after the `ここから{N}` prefix and `magnitude`
/// is `N`. `大きな` → `+N`, `小さな` → `-N`; `None` for a zero/overflowing
/// magnitude or any other tail.
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

/// Parse the line-layout clause after `ここから{N}字下げ、` (#78).
///
/// `after` is the text following `字下げ、` in the opener body. Two
/// corpus-attested forms:
///   * `{W}字詰め`          → [`IndentLayout::LineWidth`] (`W` chars per line)
///   * `{L}行{W}字組み[で]`  → [`IndentLayout::Kumi`] (`L` lines of `W` chars)
///
/// Returns `None` for anything else, so the bracket falls through to
/// `Directive{Unknown}` (round-trips byte-identical) instead of being
/// claimed in error.
/// Parse the `、`-separated clause stack following `ここから{N}字下げ、` into a
/// fully-resolved [`IndentBlock`] (#78 compound indent).
///
/// Each clause is resolved by [`resolve_indent_segment`]; the whole compound is
/// declined (`None` → generic `Unknown`, lossless) if any clause is unknown or
/// a clause repeats / conflicts. Clause order in the source is irrelevant — the
/// serializer re-emits a canonical order — so the same set always round-trips.
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

/// Fold one `字下げ、`-tail clause into `block`. Returns `None` (declining the
/// whole compound) for an unknown clause or one that conflicts with an already
/// resolved clause (a repeated wrap / layout / style — an ambiguous re-emission
/// must never arise).
fn resolve_indent_segment(segment: &str, block: &mut IndentBlock) -> Option<()> {
    // 折り返して{M}字下げ — hanging-indent continuation width.
    if let Some(rest) = segment.strip_prefix("折り返して") {
        let (m, tail) = parse_decimal_u8_prefix(rest)?;
        if tail != "字下げ" || block.wrap.is_some() {
            return None;
        }
        block.wrap = Some(m);
        return Some(());
    }
    // ページの左右中央[に] / 左右中央 / 中央揃え — page centring.
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
    // {W}字詰め / {L}行{W}字組み[で] — secondary line layout.
    if let Some(layout) = parse_indent_line_layout(segment) {
        if !matches!(block.layout, IndentLayout::None) {
            return None;
        }
        block.layout = layout;
        return Some(());
    }
    // Decorative styles (co-applied, close with the generic 字下げ終わり).
    let styles = &mut block.styles;
    match segment {
        "ゴシック体" | "ゴチック" if !styles.bold => styles.bold = true,
        "横書き" | "横組み" if !styles.horizontal => styles.horizontal = true,
        "罫囲み" | "表罫囲み" if !styles.framed => styles.framed = true,
        // 小さい活字 = one stage smaller (FontShift(-1)).
        "小さい活字" if styles.font.is_none() => {
            styles.font = Some(FontShift(NonZeroI8::new(-1)?));
        }
        _ => return None,
    }
    Some(())
}

fn parse_indent_line_layout(after: &str) -> Option<IndentLayout> {
    let (lead, rest) = parse_decimal_u8_prefix(after)?;
    let lead = NonZeroU8::new(lead)?; // folds the `lead >= 1` guard
    if rest == "字詰め" {
        return Some(IndentLayout::LineWidth(LineWidth(lead)));
    }
    // `{L}行{W}字組み[で]` — the leading number is the line count.
    let after_lines = rest.strip_prefix('行')?;
    let (width, tail) = parse_decimal_u8_prefix(after_lines)?;
    let width = NonZeroU8::new(width)?; // folds the `width >= 1` guard
    if matches!(tail, "字組み" | "字組みで") {
        return Some(IndentLayout::Kumi(Kumi { lines: lead, width }));
    }
    None
}

/// Map the trailing keyword (after `に`) to a [`BoutenKind`].
///
/// The reverse of [`BoutenKind::keyword`], derived by walking the single
/// [`BOUTEN_KINDS`] source rather than a hand-maintained second table —
/// so a mark can never be recognised in the forward direction
/// (`keyword`) yet silently missed here. `×傍点` is accepted as an input
/// alias for the canonical ばつ傍点. Unknown suffixes return `None`,
/// letting the annotation fall through to the `Directive{Unknown}`
/// catch-all. Lookup is a short linear scan (14 entries, dominated by
/// the leading-byte mismatch on the first compare).
pub(super) fn bouten_kind_from_suffix(s: &str) -> Option<BoutenKind> {
    if s == "×傍点" {
        return Some(BoutenKind::Cross);
    }
    if let Some(kind) = BOUTEN_KINDS.iter().copied().find(|k| k.keyword() == s) {
        return Some(kind);
    }
    // Marker-suffix spelling: `傍点（白丸）` / `傍点◎` — 傍点 first, then the
    // mark named parenthetically or as a bare glyph. Resolve it to the same
    // kind the canonical marker-prefix keyword (`白丸傍点`) yields. Bare
    // `傍点` already matched Goma above, so a non-empty remainder here is a
    // genuine marker.
    let marker = s.strip_prefix("傍点").filter(|m| !m.is_empty())?;
    bouten_kind_from_marker(marker)
}

/// Resolve the mark named in the `傍点（白丸）` / `傍点◎` suffix spelling to a
/// [`BoutenKind`]. Parenthetical names are derived DRY from [`BOUTEN_KINDS`]
/// (`<name>傍点` equals a kind's [`keyword`](BoutenKind::keyword)); the glyph
/// arm covers the bare-glyph spellings the corpus uses. No new [`BoutenKind`]
/// — every marker maps onto an existing variant.
fn bouten_kind_from_marker(marker: &str) -> Option<BoutenKind> {
    // `（白丸）` -> `白丸`; a bare glyph passes through unchanged.
    let name = marker
        .strip_prefix('（')
        .and_then(|inner| inner.strip_suffix('）'))
        .unwrap_or(marker);
    match name {
        "◎" => return Some(BoutenKind::DoubleCircle),
        "○" | "◯" => return Some(BoutenKind::WhiteCircle),
        "●" => return Some(BoutenKind::Circle),
        "×" | "✕" => return Some(BoutenKind::Cross),
        "△" => return Some(BoutenKind::WhiteTriangle),
        "▲" => return Some(BoutenKind::BlackTriangle),
        _ => {}
    }
    // `白丸` matches the kind whose keyword() is `白丸傍点`. Guard against an
    // empty name (`傍点（）`) matching Goma's bare `傍点` keyword.
    (!name.is_empty())
        .then(|| {
            BOUTEN_KINDS
                .iter()
                .copied()
                .find(|k| k.keyword().strip_suffix("傍点") == Some(name))
        })
        .flatten()
}

/// Parse a 傍点/傍線 range-form body into `(kind, position, is_close)`.
/// Strips an optional `左に` left-side prefix and an optional `終わり`
/// close suffix; the remainder must be a [`bouten_kind_from_suffix`]
/// keyword (all fourteen kinds, incl. the rare 鎖線 / 破線 / 黒三角傍点).
/// Returns `None` (→ `Directive{Unknown}`) for any non-bouten body.
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

/// Parse a 小書き range body into `(side, is_close)`. `行右小書き` →
/// `BoutenPosition::Right`, `行左小書き` → `Left`; an optional `終わり`
/// suffix marks the close. Returns `None` (→ `Directive{Unknown}`) for any
/// other body, so a needle-prefix-but-longer body like `行右小書きほげ`
/// declines cleanly.
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

/// Parse a 太字 / 斜体 range / block body into `(kind, block, is_close)`.
/// `block` is `true` for the `ここから…` / `ここで…終わり` block form,
/// `false` for the bare inline range `［＃太字］…［＃太字終わり］`. Returns
/// `None` (→ `Directive{Unknown}`) for any non-emphasis body.
/// Parse a キャプション range / block body into `(block, is_close)`.
/// `block` is `true` for `ここから…` / `ここで…終わり`, `false` for the bare
/// inline range `［＃キャプション］…［＃キャプション終わり］`.
fn parse_caption_body(body: &str) -> Option<(bool, bool)> {
    Some(match body {
        "キャプション" => (false, false),
        "キャプション終わり" => (false, true),
        "ここからキャプション" => (true, false),
        "ここでキャプション終わり" => (true, true),
        _ => return None,
    })
}

/// Parse an absolute font-size line body into `(size, bold)`. The body is a
/// size keyword (`特大文字` / `大文字` / `中文字` / `小文字`) optionally followed
/// by the `、太字` compound. Any other shape (a trailing run, an unknown
/// compound) declines to `Directive{Unknown}` — keeping `大文字下げ` and the like
/// out.
///
/// Only the `、太字` spelling is recognised: `bold` is a flag, not a spelling, so
/// serialization is canonical `、太字`; recognising the rarer `、ゴシック体`
/// (1 corpus occurrence) would lose its spelling and break the verbatim
/// round-trip — it stays `Directive{Unknown}` instead (mirrors §6.12 `この行は
/// ゴシック体` recognising a single spelling).
fn parse_line_font_size(body: &str) -> Option<(AbsoluteSize, bool)> {
    let (size, rest) = if let Some(r) = body.strip_prefix("特大文字") {
        (AbsoluteSize::ExtraLarge, r)
    } else if let Some(r) = body.strip_prefix("大文字") {
        (AbsoluteSize::Large, r)
    } else if let Some(r) = body.strip_prefix("中文字") {
        (AbsoluteSize::Medium, r)
    } else if let Some(r) = body.strip_prefix("小文字") {
        (AbsoluteSize::Small, r)
    } else {
        return None;
    };
    let bold = match rest {
        "" => false,
        "、太字" => true,
        _ => return None,
    };
    Some((size, bold))
}

/// Parse a 縦中横 paired-range body into `is_close`. `縦中横` opens, `縦中横
/// 終わり` closes; any other body (incl. the longer `縦中横ほげ`) declines to
/// `Directive{Unknown}`.
fn parse_tcy_range_body(body: &str) -> Option<bool> {
    match body {
        "縦中横" => Some(false),
        "縦中横終わり" => Some(true),
        _ => None,
    }
}

/// Parse a 太字 / 斜体 range / block body into `(is_italic, padded, is_close)`.
/// `padded` is `true` for the `ここから…` / `ここで…終わり` block form, `false`
/// for the bare inline range. Returns `None` (→ `Directive{Unknown}`) for any
/// non-emphasis body.
pub(super) fn parse_emphasis_body(body: &str) -> Option<(bool, bool, bool)> {
    Some(match body {
        "太字" | "ゴシック体" | "ゴチック" => (false, false, false),
        "太字終わり" | "ゴシック体終わり" | "ゴチック終わり" => {
            (false, false, true)
        }
        "ここから太字" | "ここからゴシック体" | "ここからゴチック" => {
            (false, true, false)
        }
        "ここで太字終わり" | "ここでゴシック体終わり" | "ここでゴチック終わり" => {
            (false, true, true)
        }
        "斜体" => (true, false, false),
        "斜体終わり" => (true, false, true),
        "ここから斜体" => (true, true, false),
        "ここで斜体終わり" => (true, true, true),
        _ => return None,
    })
}

/// Parse a leading run of ASCII / full-width decimal digits into a
/// [`u8`] and return the remainder slice.
///
/// Returns `None` if the leading char is not a digit, or if the value
/// overflows `u8` (> 255). `saturating_mul` / `saturating_add` during
/// accumulation keep the `u32` intermediate bounded, but the final
/// `try_from` enforces the `u8` range — a body like `300字下げ` fails
/// cleanly rather than wrapping to 44.
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
        value = value.saturating_mul(10).saturating_add(d);
        consumed = idx + ch.len_utf8();
    }
    if consumed == 0 {
        return None;
    }
    let value_u8 = u8::try_from(value).ok()?;
    Some((value_u8, &s[consumed..]))
}
