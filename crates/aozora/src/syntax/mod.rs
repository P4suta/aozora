//! AST type definitions for the aozora parser.
//!
//! # AST shape
//!
//! The **sole AST** is the owned AST defined in [`mod@ast`]:
//! lifetime-free, `Copy`-able nodes whose string and run payloads are
//! `u32` handles into a flat `NodeStore` (a string interner plus
//! content / segment pools), deduplicated through the store's interner.
//! Public consumers (`aozora` meta crate, FFI / WASM / Python drivers,
//! CLI) parse via `aozora::Document::parse()` and walk the owned
//! `ast::Node` values.
//!
//! # Top-level surface
//!
//! Only the **shared `Copy`-able payloads** referenced by the owned
//! AST (`BoutenKind`, `BoutenPosition`, `Container`, `SectionKind`,
//! `HeadingKind`, `HeadingStyle`, `MarginNoteKind`, `RubySide`,
//! `DirectiveKind`) live at the top level. The attribute × scope
//! formatting model (`Format` / `ForwardAttr` / `LineFormat` /
//! `RegionFormat` / `RegionClose` and their `NonZero` parameters) lives
//! under [`mod@format`]. The owned AST node types live under
//! [`mod@ast`]; the builder under [`mod@alloc`].

#![forbid(unsafe_code)]

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
pub mod alloc;
pub mod ast;
pub mod degraded;
pub mod format;
pub mod lint;
pub mod node_kind;

pub use format::{
    AbsoluteSize, AccentMark, BlockStyles, ColumnCount, EnclosureKind, FontShift, Format,
    ForwardAttr, ForwardOrigin, IndentBlock, IndentLayout, Kumi, LineFormat, LineWidth,
    RegionClose, RegionFormat,
};
pub use node_kind::NodeKind;

/// The typed canonical value of a gaiji reference and its building
/// blocks, re-exported from [`crate::encoding::gaiji`].
///
/// [`GaijiCanonical`] is the structured replacement for the former
/// `(ucs, mencode)` pair on the gaiji node; [`MenKuTen`] is its
/// `第N水準P-K-T` payload and [`Resolved`] the resolved-glyph result.
pub use crate::encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

/// Byte-range span into the original source document.
///
/// Re-exported from [`crate::spec::Span`] — see that module for the
/// canonical definition.
pub use crate::spec::Span;

/// Paired block container payload: carries only the kind descriptor.
///
/// Children live in the AST as the container node's children
/// (the `post_process` paired-container splice reparents them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct Container {
    /// Which container family this open marker begins. The open
    /// [`RegionFormat`] payload is authoritative when the pair round-trips.
    pub kind: RegionFormat,
}

/// Which 傍点 (emphasis dot) or 傍線 (sideline) mark decorates a run.
///
/// Carried by both the forward-reference `ast::ForwardFormat` leaf and the
/// paired [`crate::syntax::RegionFormat::Bouten`]. The 点 (dot) vs 線 (line) split —
/// see [`Self::is_line`] — is the family boundary the
/// `mismatched_bouten_container` diagnostic enforces. Each variant maps to a
/// canonical 青空文庫 keyword via [`Self::keyword`]; [`BOUTEN_KINDS`] is the
/// single declaration-order list the rest of the workspace derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    /// ゴマ
    Goma,
    /// 白ゴマ
    WhiteSesame,
    /// 丸
    Circle,
    /// 白丸
    WhiteCircle,
    /// 二重丸
    DoubleCircle,
    /// 蛇の目
    Janome,
    /// ばつ
    Cross,
    /// 白三角
    WhiteTriangle,
    /// 波線
    WavyLine,
    /// 傍線
    UnderLine,
    /// 二重傍線
    DoubleUnderLine,
    /// 鎖線
    ChainLine,
    /// 破線
    DashedLine,
    /// 黒三角
    BlackTriangle,
}

impl BoutenKind {
    /// Whether this is a 傍線 (line) variant rather than a 傍点 (dot)
    /// variant. The 点/線 split is the *family* boundary used by
    /// `mismatched_bouten_container`: a `［＃傍点］` range closed by a
    /// `［＃傍線終わり］` (or vice-versa) is the mismatch the diagnostic
    /// reports.
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

    /// Stable family tag (`"傍点"` / `"傍線"`) for diagnostics that name a
    /// mismatched bouten range pair.
    #[must_use]
    pub const fn family_str(self) -> &'static str {
        if self.is_line() { "傍線" } else { "傍点" }
    }
}

/// Every [`BoutenKind`] variant, in declaration order.
///
/// The single enumeration source the rest of the workspace derives from:
/// the parser's reverse keyword→kind lookup walks this list against
/// [`BoutenKind::keyword`] instead of hand-maintaining a second match,
/// and the render / spec slug tables are drift-checked against it. Adding
/// a bouten mark therefore means a new variant + its `keyword` arm + one
/// row here — nothing else can silently fall out of sync.
pub const BOUTEN_KINDS: &[BoutenKind] = &[
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

/// Which side of the vertical-writing base text the bouten marks sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    /// Right of (in horizontal terms, above) the base text — the
    /// default side, the bare `［＃「X」に傍点］` form.
    #[default]
    Right,
    /// Left of (below) the base text — the `左に` modifier
    /// (`［＃「X」の左に傍点］`).
    Left,
    /// Both sides of the base text — the `の両側に` modifier
    /// (`［＃「X」の両側に傍線］`).
    Both,
}

/// Which side of the base text a ruby reading sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RubySide {
    /// Standard ruby — `｜base《reading》` (right of / above the base).
    #[default]
    Right,
    /// Left-side (below) ruby — `［＃「base」の左に「reading」のルビ］`, the
    /// saidoku-moji (再読文字) building block.
    Left,
}

/// The character class an implicit-ruby base run belongs to.
///
/// A bare `《reading》` attaches to the maximal run of a *single* class
/// immediately preceding it — mixing classes would move where the base
/// starts on re-parse, so a run stays within one class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RubyBaseClass {
    /// CJK ideographs (main block + Ext A/B..F + compatibility) plus the
    /// iteration marks `々` / `〆` — the historical base set.
    Kanji,
    /// Hiragana (letters + `ゝ` / `ゞ`).
    Hiragana,
    /// Katakana (letters + `ー` / `ヽ` / `ヾ`), excluding the small `ヵ` /
    /// `ヶ` (see [`ruby_base_class`]).
    Katakana,
    /// Basic + fullwidth Latin letters.
    Latin,
    /// Greek letters.
    Greek,
}

/// Classify a character for implicit-ruby base attachment, or `None` when
/// it cannot start or extend a base run.
///
/// Single source of truth shared by the classifier's implicit-base walk
/// ([`is_ruby_base_char`]'s callers) and the serializer's canonical
/// bare-vs-`｜` decision (ADR 0002): the serializer drops `｜` only when a
/// bare reading would re-parse to the *same* base, decided by comparing
/// classes here — so both sides must move in lockstep.
///
/// The small katakana `ヵ` (U+30F5) and `ヶ` (U+30F6) are deliberately
/// excluded from every class. Treating them as `Kanji` would over-grow a
/// base such as `数ヶ所《かしょ》` (base `所` → `数ヶ所`, wrong reading);
/// treating them as `Katakana` would put the base on the `ヶ` itself — so
/// `六ヶ《むつか》` correctly *declines* instead of guessing.
#[must_use]
pub const fn ruby_base_class(ch: char) -> Option<RubyBaseClass> {
    Some(match ch {
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆' => RubyBaseClass::Kanji,
        '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' => RubyBaseClass::Hiragana,
        // Katakana letters ァ..ヴ, phonetic ヷ..ヺ, and ー / ヽ / ヾ — but
        // NOT the small ヵ(30F5) / ヶ(30F6) or the middle dot ・(30FB).
        '\u{30A1}'..='\u{30F4}' | '\u{30F7}'..='\u{30FA}' | '\u{30FC}'..='\u{30FE}' => {
            RubyBaseClass::Katakana
        }
        'A'..='Z' | 'a'..='z' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' => RubyBaseClass::Latin,
        '\u{0391}'..='\u{03A9}' | '\u{03B1}'..='\u{03C9}' => RubyBaseClass::Greek,
        _ => return None,
    })
}

/// True when `ch` can serve as (part of) an implicit-ruby *kanji* base —
/// the historical predicate, now expressed via [`ruby_base_class`]. Byte
/// for byte identical to the pre-`RubyBaseClass` definition.
#[must_use]
pub const fn is_ruby_base_char(ch: char) -> bool {
    matches!(ruby_base_class(ch), Some(RubyBaseClass::Kanji))
}

/// Which annotation flavour an `ast::MarginNote` carries.
///
/// 注記 and 傍記 share the `MarginNote` structure (a note attached to a
/// preceding run) but round-trip to distinct keywords, so the flavour is
/// preserved here even though both render the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MarginNoteKind {
    /// 注記 — `［＃「X」の左に「Y」の注記］`, a left-side editorial gloss.
    #[default]
    Gloss,
    /// 傍記 — `［＃「X」に「Y」の傍記］`, a redaction marker (典型的に ×)
    /// written beside X, used in censorship restoration.
    Marginal,
}

impl MarginNoteKind {
    /// The `(connector, suffix)` source literals that wrap the note text
    /// when an `ast::MarginNote` of this flavour round-trips
    /// back to source as `base［＃「base{connector}note{suffix}`.
    ///
    /// Renderers call this instead of matching the (`non_exhaustive`)
    /// variants, so a future flavour must add its affixes here — keeping
    /// the round-trip vocabulary beside the variant definition.
    #[must_use]
    pub const fn serialize_affixes(self) -> (&'static str, &'static str) {
        match self {
            // 注記 normalises bare `に` input to the canonical `の左に…の注記`.
            Self::Gloss => ("」の左に「", "」の注記］"),
            // 傍記 keeps the bare `に` — there is no 左 in the source.
            Self::Marginal => ("」に「", "」の傍記］"),
        }
    }
}

/// Which section-break directive an `ast::Node::SectionBreak` carries —
/// the stronger page-structure breaks beyond the plain `［＃改ページ］`.
///
/// Each variant maps to its canonical keyword via [`Self::keyword`];
/// [`SECTION_KINDS`] is the declaration-order list the renderer derives its
/// class list from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SectionKind {
    /// `［＃改丁］`
    Kaicho,
    /// `［＃改段］`
    Kaidan,
    /// `［＃改見開き］`
    Kaimihiraki,
}

/// Heading *level* — the 大 / 中 / 小 outline rank.
///
/// Orthogonal to [`HeadingStyle`]; the two combine (同行中見出し is
/// `Medium` + `SameLine`, 窓小見出し is `Small` + `Window`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingKind {
    /// 大見出し — the top outline level (renders as `<h1>`).
    Large,
    /// 中見出し — the middle outline level (renders as `<h2>`).
    Medium,
    /// 小見出し — the lowest outline level (renders as `<h3>`).
    Small,
}

impl HeadingKind {
    /// The numeric outline level — `1` = 大, `2` = 中, `3` = 小 — carried by
    /// the inline `ast::HeadingHint`'s `data-level` attribute.
    ///
    /// The single source of the 大/中/小 → 1/2/3 mapping (the renderer and the
    /// classifier both key on this instead of an ad-hoc local table).
    #[must_use]
    pub const fn outline_level(self) -> u8 {
        match self {
            Self::Medium => 2,
            Self::Small => 3,
            // 大見出し and any future top level default to 1.
            _ => 1,
        }
    }
}

/// Heading *style* — standard, 同行 (same-line), or 窓 (window).
///
/// Orthogonal to [`HeadingKind`] (the 大 / 中 / 小 level): each style
/// pairs with any level. The 同行 style runs the title into the body on the
/// same line; 窓 is an inset title. 副見出し is **not** a real annotation (it
/// does not occur in the corpus) and is deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingStyle {
    /// Standard heading — no 同行 / 窓 prefix. The default.
    #[default]
    Standard,
    /// 同行見出し — the title runs into the body on the same line.
    SameLine,
    /// 窓見出し — an inset ("window") title.
    Window,
}

/// Every [`SectionKind`] variant in declaration order.
///
/// Drives the renderer's class-list derivation (and any codegen) so a new
/// section break flows in without a hand-maintained parallel — mirrors
/// [`BOUTEN_KINDS`].
pub const SECTION_KINDS: &[SectionKind] = &[
    SectionKind::Kaicho,
    SectionKind::Kaidan,
    SectionKind::Kaimihiraki,
];

/// Every [`HeadingKind`] outline level in declaration order. See
/// [`BOUTEN_KINDS`].
pub const HEADING_KINDS: &[HeadingKind] =
    &[HeadingKind::Large, HeadingKind::Medium, HeadingKind::Small];

/// Every [`HeadingStyle`] in declaration order. See [`BOUTEN_KINDS`].
pub const HEADING_STYLES: &[HeadingStyle] = &[
    HeadingStyle::Standard,
    HeadingStyle::SameLine,
    HeadingStyle::Window,
];

// --- enum → canonical 青空文庫 keyword ---------------------------------------
//
// The single source of truth for the Japanese keyword each render-bearing
// enum maps to (e.g. `BoutenKind::WhiteSesame` → "白ゴマ傍点"). Both the
// serializer (AST → annotation text) and the renderers key on these, and
// `crate::spec::roman_slug` turns the keyword into the romaji CSS slug — so
// the keyword lives here once instead of being copied per crate.

impl BoutenKind {
    /// Canonical 青空文庫 keyword (the body of `［＃「…」に〈keyword〉］`).
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
            // Goma (無印) and any future kind default to the bare 傍点.
            _ => "傍点",
        }
    }
}

impl SectionKind {
    /// Canonical 青空文庫 keyword for the section break. Matched
    /// exhaustively: adding a variant is a compile error here until its
    /// keyword is supplied, rather than silently falling through a
    /// `#[non_exhaustive]` `_` arm.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Kaicho => "改丁",
            Self::Kaidan => "改段",
            Self::Kaimihiraki => "改見開き",
        }
    }
}

/// Classifies a generic `ast::Directive` annotation that no more
/// specific node recogniser claimed.
///
/// [`Unknown`](Self::Unknown) is the catch-all for Aozora-shaped `［＃…］`
/// notation the parser does not model; the remaining variants tag the
/// handful of annotations kept as raw `Directive`s (sic markers, warichu
/// delimiters, the header 凡例 `［＃］`, …) so consumers can act on them
/// without re-parsing the raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DirectiveKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// `［＃「」」はママ］`-style editorial *sic* marker (text reproduced as in the source).
    Sic,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    BaseTextVariant,
    /// Inline warichu opener — `［＃割り注］`.
    WarichuOpen,
    /// Inline warichu closer — `［＃割り注終わり］`.
    WarichuClose,
    /// An empty directive `［＃］` (or whitespace-only `［＃　］`). Not an
    /// unrecognised notation: it is the de-facto-standard symbol used in the
    /// file-header 凡例 line `［＃］：入力者注…` that prefixes essentially every
    /// 青空文庫 work. Typed distinctly so it leaves the `Unknown` bucket while
    /// still round-tripping its raw bytes.
    Empty,
    /// Numbered input-typist note (`［＃入力者注(N)］`) — a reference to the
    /// numbered note listed in the file's 凡例. Rendered as a visible `注N`
    /// superscript; typed distinctly so it leaves the `Unknown` bucket.
    EditorNote,
    /// Ruby-presence editorial note (`［＃「X」にルビ］`) — records that the run
    /// `X` carries a ruby gloss in the source. The gloss text itself is not in
    /// the directive; this is a proofreading marker, not renderable ruby.
    /// Rendered as a compact visible marker; the raw bracket round-trips.
    RubyAttached,
    /// Ruby-binding editorial note (`［＃ルビは「X」にかかる］`) — records that a
    /// nearby ruby applies to the run `X` (disambiguation, not renderable
    /// ruby). Rendered as a compact marker; the raw bracket round-trips.
    RubyRetarget,
    /// Left-side-ruby span opener (`［＃左にルビ付き］`) — marks the start of a
    /// run that carries a left-side ruby whose reading is named on the matching
    /// [`Self::RubyPairClose`]. A raw-preserving `Direct` directive (the pair is
    /// not coupled at the splice layer, like the inline warichu pair).
    RubyPairOpen,
    /// Left-side-ruby span closer (`［＃左に「Y」のルビ付き終わり］`) — names the
    /// left-side ruby reading `Y` for the span opened by [`Self::RubyPairOpen`].
    /// `Y` is preserved verbatim in the raw bracket and shown in the marker.
    RubyPairClose,
    /// Margin-note span opener (`［＃注記付き］` / `［＃左に注記付き］`) — marks the
    /// start of a run that carries a margin note whose text is named on the
    /// matching [`Self::MarginNotePairClose`]. A raw-preserving `Direct`
    /// directive (the pair is not coupled at the splice layer, like the inline
    /// warichu pair). The `左に` prefix records that the note sits on the left.
    MarginNotePairOpen,
    /// Margin-note span closer (`［＃「Y」の注記付き終わり］` / `［＃左に「Y」の注記
    /// 付き終わり］`) — names the margin-note text `Y` for the span opened by
    /// [`Self::MarginNotePairOpen`]. `Y` is preserved verbatim in the raw
    /// bracket (it may contain a nested `［＃…］` gaiji) and shown in the marker.
    MarginNotePairClose,
}

/// Parse- and render-time error surface for syntax-layer consumers.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    /// A node-kind tag string did not resolve to a known node kind. The
    /// offending tag is carried verbatim in [`kind`](Self::UnknownKind::kind)
    /// and echoed in the `未知のノード種別です` message; the diagnostic code is
    /// `aozora::syntax::unknown_kind`.
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(aozora::syntax::unknown_kind))]
    UnknownKind {
        /// The unrecognised tag string, as received.
        kind: Box<str>,
    },
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
        use RubyBaseClass::{Greek, Hiragana, Kanji, Katakana, Latin};
        // Table-driven: every class plus the *endpoint* char of each Unicode
        // range it unions, so a shrunk/dropped range arm (or a `-> None`) is
        // caught. Endpoints are written as `\u{..}` to pin the exact bounds.
        let cases: &[(char, RubyBaseClass)] = &[
            // Kanji — CJK Ext A / main / compat / Ext B..F endpoints + 々〆.
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
            // Hiragana — letters ぁ..ゖ and ゝ / ゞ.
            ('\u{3041}', Hiragana),
            ('\u{3096}', Hiragana),
            ('\u{309D}', Hiragana),
            ('\u{309E}', Hiragana),
            ('あ', Hiragana),
            // Katakana — ァ..ヴ, ヷ..ヺ, and ー / ヽ / ヾ.
            ('\u{30A1}', Katakana),
            ('\u{30F4}', Katakana),
            ('\u{30F7}', Katakana),
            ('\u{30FA}', Katakana),
            ('\u{30FC}', Katakana),
            ('\u{30FE}', Katakana),
            ('カ', Katakana),
            // Latin — basic + fullwidth letters, both cases.
            ('A', Latin),
            ('Z', Latin),
            ('a', Latin),
            ('z', Latin),
            ('Ａ', Latin),
            ('Ｚ', Latin),
            ('ａ', Latin),
            ('ｚ', Latin),
            // Greek — upper Α..Ω and lower α..ω.
            ('\u{0391}', Greek),
            ('\u{03A9}', Greek),
            ('\u{03B1}', Greek),
            ('\u{03C9}', Greek),
        ];
        for &(ch, class) in cases {
            assert_eq!(
                ruby_base_class(ch),
                Some(class),
                "U+{:04X} should classify as {class:?}",
                ch as u32
            );
        }
        // Chars that must NOT classify: the deliberately-excluded small
        // katakana / middle-dot, and just-outside-range neighbours of every
        // block, so no arm over-grows.
        let none_cases: &[char] = &[
            'ヵ',       // U+30F5 — deliberately excluded (see doc comment)
            'ヶ',       // U+30F6 — deliberately excluded
            '・',       // U+30FB — middle dot, excluded
            '\u{33FF}', // just below Kanji Ext A
            '\u{3040}', // just below Hiragana
            '\u{3097}', // between the two Hiragana sub-ranges (unassigned)
            '\u{30FF}', // just above Katakana
            '\u{0390}', // just below Greek upper
            '\u{03AA}', // between Greek upper and lower
            '\u{03CA}', // just above Greek lower
            ' ', '5', '。',
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
        // The historical kanji predicate: true iff `ruby_base_class` is Kanji.
        assert!(is_ruby_base_char('漢'));
        assert!(is_ruby_base_char('々'));
        assert!(is_ruby_base_char('\u{4E00}'));
        // Every non-kanji base class, and a non-base char, are false.
        assert!(!is_ruby_base_char('あ')); // hiragana
        assert!(!is_ruby_base_char('カ')); // katakana
        assert!(!is_ruby_base_char('A')); // latin
        assert!(!is_ruby_base_char('\u{0391}')); // greek
        assert!(!is_ruby_base_char(' ')); // not a base at all
    }

    #[test]
    fn margin_note_kind_serialize_affixes_are_exact() {
        // The `(connector, suffix)` literals that wrap a round-tripped note —
        // pinned byte-exact so a stubbed `("", "")` / `("xyzzy", …)` is caught.
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
        // The 大/中/小 → 1/2/3 mapping; each rank pinned so a stubbed constant
        // or a dropped Medium/Small arm is caught.
        assert_eq!(HeadingKind::Large.outline_level(), 1);
        assert_eq!(HeadingKind::Medium.outline_level(), 2);
        assert_eq!(HeadingKind::Small.outline_level(), 3);
    }

    #[test]
    fn bouten_keyword_is_exhaustive_and_stable() {
        // Every named variant keys its canonical 青空文庫 keyword; the
        // bare 傍点 (Goma) flows through the `_` default arm.
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
        // 線 family.
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
        // 点 family.
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
            // FontSize carries a magnitude and falls through to 太字.
            (
                ForwardAttr::FontSize(FontShift(NonZeroI8::new(3).unwrap())),
                "太字",
            ),
            // AccentDot's body is the selector grammar (serialized from the
            // interned `accent_body`, never `keyword()`), so it rides the 太字
            // default too — `keyword()` is never called for it.
            (ForwardAttr::AccentDot, "太字"),
            // Accent's suffix carries the bracketed mark symbol (serialized in a
            // dedicated arm, never `keyword()`), so it rides the 太字 default too.
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
        // Cheap structural smoke: every level pairs with every style and
        // the payload structs are Copy + Eq as the AST relies on.
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
        // The spec's RENDER_SLUGS must carry a romaji slug for every bouten
        // kind so the renderer's `aozora-bouten-<slug>` class never falls
        // back. Drift-guards the syntax↔spec bouten tables against the
        // single `BOUTEN_KINDS` source.
        for k in BOUTEN_KINDS {
            assert!(
                roman_slug(k.keyword()).is_some(),
                "RENDER_SLUGS missing a slug for bouten keyword {:?}",
                k.keyword()
            );
        }
    }
}
