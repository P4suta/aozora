//! Canonical slug catalogue for Aozora annotation bodies (Phase 1.2 of
//! the editor-integration sprint).
//!
//! `classify::BODY_PATTERNS` is the *parser-side* aho-corasick
//! table — its goal is exhaustive matching, including every digit a
//! `{N}字下げ` form can start with. This module is the *editor-side*
//! mirror: the public, stable list of slugs an LSP completion menu
//! offers and the LSP `canonicalize` code action snaps user input to.
//!
//! The two tables stay in sync via the
//! `every_slug_dispatches_in_phase3_body_dispatcher` integration test
//! living in `aozora-pipeline`.
//!
//! ## Why a separate table
//!
//! - **Granularity**: the editor wants `{N}字下げ` (one entry, accepts
//!   a parameter), not ten distinct rows for each digit prefix.
//! - **Documentation**: each entry carries a Japanese `doc` string that
//!   becomes the LSP completion item's `documentation` field.
//! - **Stability**: `BODY_PATTERNS`'s exact shape is tied to classify-stage
//!   internals (`LeftmostLongest` dispatch order, `BodyFamily` variants);
//!   downstream editor consumers should not depend on it.
//!
//! ## Canonicalisation
//!
//! [`canonicalise_slug`] maps a known orthographic *variant* (typically
//! a hiragana-only spelling — `ぼうてん`, `にぼうてん`) to the canonical
//! form (`傍点`). The variant table is intentionally small: it covers
//! the highest-frequency author-side abbreviations the editor surface
//! treats as a one-keystroke-quick-fix. Any input that is already
//! canonical short-circuits with `Some(canonical)` so callers can
//! always trust the return value.

use crate::PairKind;

/// Family / coarse category a slug belongs to. Used by the LSP
/// completion UI to group entries (`CompletionItem::sort_text`) and
/// pick an appropriate `CompletionItemKind` icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SlugFamily {
    /// `［＃改ページ］` and other section-level page breaks.
    PageBreak,
    /// `［＃改丁］`, `［＃改段］`, `［＃改見開き］`.
    Section,
    /// Block container open marker (`［＃ここから...］`). Pairs with
    /// the corresponding [`SlugFamily::BlockContainerClose`] slug.
    BlockContainerOpen,
    /// Block container close marker (`［＃ここで...終わり］`).
    BlockContainerClose,
    /// Leaf-line layout slug applied to the immediately preceding
    /// paragraph (`地付き`, `{N}字下げ`, `地から{N}字上げ`).
    LeafAlign,
    /// Forward-reference bouten / underline (`［＃「target」に傍点］`).
    Bouten,
    /// Inline figure (`［＃挿絵（path）入る］`).
    Illustration,
    /// Framed rule frame (open / close).
    Framed,
    /// Warichu inline-break (open / close).
    Warichu,
    /// Forward-reference 縦中横 (`［＃「target」は縦中横］`).
    CombineUpright,
    /// Kaeriten single mark (一, 二, 三, 上, 中, 下, 甲, 乙, 丙, 丁,
    /// 四, レ).
    KaeritenSingle,
    /// Kaeriten compound mark (一レ, 二レ, …).
    KaeritenCompound,
}

impl SlugFamily {
    /// Every variant in declaration order. Lets the wire-tag
    /// exhaustiveness test and codegen enumerate the families without a
    /// hand-maintained parallel.
    pub const ALL: [Self; 12] = [
        Self::PageBreak,
        Self::Section,
        Self::BlockContainerOpen,
        Self::BlockContainerClose,
        Self::LeafAlign,
        Self::Bouten,
        Self::Illustration,
        Self::Framed,
        Self::Warichu,
        Self::CombineUpright,
        Self::KaeritenSingle,
        Self::KaeritenCompound,
    ];

    /// Stable camelCase identifier used by the driver wire formats.
    /// Centralised in the enum's own crate so the wire spelling has a
    /// single authority and the `match` is exhaustiveness-checked at
    /// compile time (no `_` fallback — a new family must declare its tag).
    #[must_use]
    pub const fn as_wire_tag(self) -> &'static str {
        match self {
            Self::PageBreak => "pageBreak",
            Self::Section => "section",
            Self::BlockContainerOpen => "blockContainerOpen",
            Self::BlockContainerClose => "blockContainerClose",
            Self::LeafAlign => "leafAlign",
            Self::Bouten => "bouten",
            Self::Illustration => "illustration",
            Self::Framed => "framed",
            Self::Warichu => "warichu",
            Self::CombineUpright => "combineUpright",
            Self::KaeritenSingle => "kaeritenSingle",
            Self::KaeritenCompound => "kaeritenCompound",
        }
    }
}

/// One row of the slug catalogue.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SlugEntry {
    /// Canonical body text (without the surrounding `［＃` / `］`).
    pub canonical: &'static str,
    /// Coarse category / family.
    pub family: SlugFamily,
    /// Whether the slug expects a numeric parameter (or, for `Illustration`,
    /// a path) following the canonical text.
    pub accepts_param: bool,
    /// Short Japanese description shown in LSP `CompletionItem.detail`
    /// / `documentation` fields. Single sentence, no surrounding
    /// punctuation, terminating period.
    pub doc: &'static str,
    /// For `BlockContainerOpen` / `BlockContainerClose` slugs, the
    /// canonical text of the partner slug — so the editor can link
    /// them together (insert close on accept, jump to partner, …).
    /// `None` for non-paired families.
    pub partner: Option<&'static str>,
    /// Always [`PairKind::Bracket`] for the slugs in this table — the
    /// surrounding `［＃ … ］` is a bracket pair. Carried so editor
    /// snippets can render the wrapper bracket pair without having to
    /// re-derive it.
    pub wrapper: PairKind,
}

/// Canonical slug catalogue. See module docs.
///
/// Order is irrelevant for behavior; entries are grouped by family for
/// readability. The `every_canonical_resolves_through_canonicalise_slug`
/// test pins identity round-trip for every entry.
pub const SLUGS: &[SlugEntry] = &[
    // --- Section / page break ----------------------------------------------
    SlugEntry {
        canonical: "改ページ",
        family: SlugFamily::PageBreak,
        accepts_param: false,
        doc: "ページを改める",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "改丁",
        family: SlugFamily::Section,
        accepts_param: false,
        doc: "改丁（次の奇数ページから）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "改段",
        family: SlugFamily::Section,
        accepts_param: false,
        doc: "改段（段組を改める）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "改見開き",
        family: SlugFamily::Section,
        accepts_param: false,
        doc: "改見開き（次の見開きへ）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    // --- Block containers (open / close pairs) -----------------------------
    SlugEntry {
        canonical: "ここから字下げ",
        family: SlugFamily::BlockContainerOpen,
        accepts_param: false,
        doc: "1字下げを開始（終わりまで）",
        partner: Some("ここで字下げ終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ここから{N}字下げ",
        family: SlugFamily::BlockContainerOpen,
        accepts_param: true,
        doc: "N字下げを開始（終わりまで）",
        partner: Some("ここで字下げ終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ここで字下げ終わり",
        family: SlugFamily::BlockContainerClose,
        accepts_param: false,
        doc: "字下げブロックを閉じる",
        partner: Some("ここから字下げ"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ここから地付き",
        family: SlugFamily::BlockContainerOpen,
        accepts_param: false,
        doc: "地付きを開始",
        partner: Some("ここで地付き終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ここから地から{N}字上げ",
        family: SlugFamily::BlockContainerOpen,
        accepts_param: true,
        doc: "地からN字上げを開始",
        partner: Some("ここで地付き終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ここで地付き終わり",
        family: SlugFamily::BlockContainerClose,
        accepts_param: false,
        doc: "地付きブロックを閉じる",
        partner: Some("ここから地付き"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "罫囲み",
        family: SlugFamily::Framed,
        accepts_param: false,
        doc: "罫線で囲む（終わりまで）",
        partner: Some("罫囲み終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "罫囲み終わり",
        family: SlugFamily::Framed,
        accepts_param: false,
        doc: "罫囲みを閉じる",
        partner: Some("罫囲み"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "割り注",
        family: SlugFamily::Warichu,
        accepts_param: false,
        doc: "割り注を開始（終わりまで）",
        partner: Some("割り注終わり"),
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "割り注終わり",
        family: SlugFamily::Warichu,
        accepts_param: false,
        doc: "割り注を閉じる",
        partner: Some("割り注"),
        wrapper: PairKind::Bracket,
    },
    // --- Leaf alignment (single-paragraph) ---------------------------------
    SlugEntry {
        canonical: "地付き",
        family: SlugFamily::LeafAlign,
        accepts_param: false,
        doc: "前の段落を地付きに揃える",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "地から{N}字上げ",
        family: SlugFamily::LeafAlign,
        accepts_param: true,
        doc: "前の段落を地からN字上げて揃える",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "{N}字下げ",
        family: SlugFamily::LeafAlign,
        accepts_param: true,
        doc: "前の段落をN字下げる（単発）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    // --- Bouten / underline (forward-ref via 「target」に...) --------------
    SlugEntry {
        canonical: "傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "ゴマ傍点（［＃「対象」に傍点］）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "白ゴマ傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "白ゴマ傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "丸傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "丸傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "白丸傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "白丸傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "二重丸傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "二重丸傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "蛇の目傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "蛇の目傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "ばつ傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "ばつ傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "白三角傍点",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "白三角傍点",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "波線",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "波線（傍線の波形）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "傍線",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "傍線（下線）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "二重傍線",
        family: SlugFamily::Bouten,
        accepts_param: false,
        doc: "二重傍線（二重下線）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    // --- Other inline -----------------------------------------------------
    SlugEntry {
        canonical: "挿絵（{path}）入る",
        family: SlugFamily::Illustration,
        accepts_param: true,
        doc: "挿絵を埋め込む",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "縦中横",
        family: SlugFamily::CombineUpright,
        accepts_param: false,
        doc: "縦中横（［＃「対象」は縦中横］）",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    // --- Kaeriten single (12) ---------------------------------------------
    SlugEntry {
        canonical: "一",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 一",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "二",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 二",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "三",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 三",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "四",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 四",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "上",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 上",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "中",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 中",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "下",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 下",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "甲",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 甲",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "乙",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 乙",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "丙",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 丙",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "丁",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 丁",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "レ",
        family: SlugFamily::KaeritenSingle,
        accepts_param: false,
        doc: "返り点 レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    // --- Kaeriten compound (6) --------------------------------------------
    SlugEntry {
        canonical: "一レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 一レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "二レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 二レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "三レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 三レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "上レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 上レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "中レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 中レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
    SlugEntry {
        canonical: "下レ",
        family: SlugFamily::KaeritenCompound,
        accepts_param: false,
        doc: "返り点 下レ",
        partner: None,
        wrapper: PairKind::Bracket,
    },
];

/// Variant → canonical mapping for [`canonicalise_slug`]. Each row
/// covers one common abbreviation or hiragana spelling that the LSP
/// snaps to the canonical form. Identity rows (`canonical → canonical`)
/// are inserted automatically by [`canonicalise_slug`] so this table
/// only needs the *non-trivial* variants.
const VARIANTS: &[(&str, &str)] = &[
    // Bouten — hiragana variants commonly typed in drafts.
    ("ぼうてん", "傍点"),
    ("にぼうてん", "傍点"),
    ("しろぼうてん", "白ゴマ傍点"),
    ("しろごまぼうてん", "白ゴマ傍点"),
    ("まるぼうてん", "丸傍点"),
    ("にまるぼうてん", "丸傍点"),
    ("しろまるぼうてん", "白丸傍点"),
    ("にしろまるぼうてん", "白丸傍点"),
    ("にじゅうまるぼうてん", "二重丸傍点"),
    ("じゃのめぼうてん", "蛇の目傍点"),
    ("ばつぼうてん", "ばつ傍点"),
    ("しろさんかくぼうてん", "白三角傍点"),
    ("はせん", "波線"),
    ("ぼうせん", "傍線"),
    ("にじゅうぼうせん", "二重傍線"),
    // Page break.
    ("かいぺーじ", "改ページ"),
    ("ページかえ", "改ページ"),
    ("かいちょう", "改丁"),
    ("かいだん", "改段"),
    ("かいみひらき", "改見開き"),
    // Block container open / close.
    ("ここからじさげ", "ここから字下げ"),
    ("ここでじさげおわり", "ここで字下げ終わり"),
    ("ここからじつき", "ここから地付き"),
    ("ここでじつきおわり", "ここで地付き終わり"),
    // Leaf align.
    ("じつき", "地付き"),
    // Other inline.
    ("たてちゅうよこ", "縦中横"),
    ("たて中横", "縦中横"),
    ("そうにゅうえ", "挿絵（{path}）入る"),
    // Framed / warichu.
    ("けいがこみ", "罫囲み"),
    ("けいがこみおわり", "罫囲み終わり"),
    ("わりちゅう", "割り注"),
    ("わりちゅうおわり", "割り注終わり"),
];

/// Snap an input slug body (with the surrounding `［＃ … ］` already
/// stripped) to the canonical form, if one is recognised.
///
/// Returns:
/// - `Some(s)` — `s` is the canonical text. `s` is `&'static str`
///   pointing into [`SLUGS`]'s `canonical` field, so callers can use
///   it as a stable key.
/// - `None` — no recognised slug. Callers may still parse `input` as a
///   `{N}字下げ` parametric form (which intentionally has no fixed
///   variant).
///
/// Identity rows are accepted: passing a canonical string back returns
/// the same pointer. This lets the LSP's `canonicalize` code action
/// short-circuit safely.
#[must_use]
pub fn canonicalise_slug(input: &str) -> Option<&'static str> {
    // Identity short-circuit. SLUGS is small (~40 entries) and the
    // strings are short, so a linear scan beats hashing on cache cost.
    for entry in SLUGS {
        if entry.canonical == input {
            return Some(entry.canonical);
        }
    }
    for &(variant, canonical) in VARIANTS {
        if variant == input {
            return Some(canonical);
        }
    }
    None
}

/// One row of the *render* slug table.
///
/// The single source of truth for the romaji CSS slug emitted for a
/// given annotation. Kept separate from [`SLUGS`] (the LSP completion
/// catalogue) because render slugs key on the enum *keyword* (傍点 kind
/// / emphasis kind / section kind), a vocabulary the completion table
/// does not carry.
///
/// `roman` is the stable kebab-case CSS slug. `reading` is the kana the
/// slug romanises (Hepburn, long vowels dropped — 改丁／かいちょう →
/// `kaicho`); `None` marks a loanword kept in English (改ページ →
/// `page-break`, キャプション → `caption`) which has no reading-derived
/// spelling to check. `jis` cites the JIS Z 8125:2004 clause the term
/// is reconciled against, where one exists.
///
/// The `render_slug_matches_reading` test re-derives Hepburn from
/// `reading` and asserts it agrees with `roman`, so a slug that drifts
/// from its reading — 小書き is こがき (→ `kogaki`), not the over-long
/// spelling once shipped — cannot reappear.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RenderSlug {
    /// Canonical Japanese keyword (matches the enum→keyword tables in
    /// `aozora-syntax`), used as the lookup key.
    pub canonical: &'static str,
    /// Kana the slug romanises, or `None` for an English loanword slug.
    pub reading: Option<&'static str>,
    /// Stable kebab-case CSS slug.
    pub roman: &'static str,
    /// JIS Z 8125:2004 clause, where the term is standardised.
    pub jis: Option<&'static str>,
}

/// The render slug catalogue — the single source of truth for romaji
/// CSS slugs. See [`RenderSlug`].
pub const RENDER_SLUGS: &[RenderSlug] = &[
    // --- Section / page break (JIS Z 8125:2004 §07.24) ---------------------
    RenderSlug {
        canonical: "改丁",
        reading: Some("かいちょう"),
        roman: "kaicho",
        jis: Some("07.24.12"),
    },
    RenderSlug {
        canonical: "改段",
        reading: Some("かいだん"),
        roman: "kaidan",
        jis: Some("07.24.14"),
    },
    RenderSlug {
        canonical: "改見開き",
        reading: Some("かいみひらき"),
        roman: "kaimihiraki",
        jis: Some("07.24.01"),
    },
    RenderSlug {
        canonical: "改ページ",
        reading: None,
        roman: "page-break",
        jis: Some("07.24.13"),
    },
    // --- Bouten kinds (圏点 JIS Z 8125:2004 §07.16) ------------------------
    RenderSlug {
        canonical: "傍点",
        reading: Some("ごま"),
        roman: "goma",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "白ゴマ傍点",
        reading: Some("しろごま"),
        roman: "shirogoma",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "丸傍点",
        reading: Some("まる"),
        roman: "maru",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "白丸傍点",
        reading: Some("しろまる"),
        roman: "shiromaru",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "二重丸傍点",
        reading: Some("にじゅうまる"),
        roman: "nijumaru",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "蛇の目傍点",
        reading: Some("じゃのめ"),
        roman: "janome",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "ばつ傍点",
        reading: Some("ばつ"),
        roman: "batsu",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "白三角傍点",
        reading: Some("しろさんかく"),
        roman: "shirosankaku",
        jis: Some("07.16"),
    },
    RenderSlug {
        canonical: "波線",
        reading: Some("なみせん"),
        roman: "namisen",
        jis: None,
    },
    RenderSlug {
        canonical: "傍線",
        reading: Some("ぼうせん"),
        roman: "bosen",
        jis: None,
    },
    RenderSlug {
        canonical: "二重傍線",
        reading: Some("にじゅうぼうせん"),
        roman: "nijubosen",
        jis: None,
    },
    RenderSlug {
        canonical: "鎖線",
        reading: Some("くさりせん"),
        roman: "kusarisen",
        jis: None,
    },
    RenderSlug {
        canonical: "破線",
        reading: Some("はせん"),
        roman: "hasen",
        jis: None,
    },
    RenderSlug {
        canonical: "黒三角傍点",
        reading: Some("くろさんかく"),
        roman: "kurosankaku",
        jis: Some("07.16"),
    },
    // --- Emphasis / inline ---------------------------------------------------
    // These keep their established English slugs: they are translations,
    // not misreadings, and span three emit paths (forward / range / block)
    // plus the `aozora-container-*` class family, so reading-based renaming
    // is tracked separately. `reading: None` keeps them out of the
    // Hepburn-consistency check. Only 行右/行左小書き carry a reading —
    // their slug once carried an over-long misspelling, now corrected
    // to `kogaki` (小書き＝こがき).
    RenderSlug {
        canonical: "斜体",
        reading: None,
        roman: "italic",
        jis: None,
    },
    RenderSlug {
        canonical: "太字",
        reading: None,
        roman: "bold",
        jis: None,
    },
    RenderSlug {
        canonical: "上付き小文字",
        reading: None,
        roman: "superscript",
        jis: Some("07.12.01"),
    },
    RenderSlug {
        canonical: "下付き小文字",
        reading: None,
        roman: "subscript",
        jis: Some("07.12.02"),
    },
    RenderSlug {
        canonical: "行右小書き",
        reading: Some("こがき"),
        roman: "kogaki-right",
        jis: None,
    },
    RenderSlug {
        canonical: "行左小書き",
        reading: Some("こがき"),
        roman: "kogaki-left",
        jis: None,
    },
    RenderSlug {
        canonical: "罫囲み",
        reading: None,
        roman: "keigakomi-inline",
        jis: None,
    },
    RenderSlug {
        canonical: "横組み",
        reading: None,
        roman: "horizontal",
        jis: None,
    },
    RenderSlug {
        canonical: "キャプション",
        reading: None,
        roman: "caption",
        jis: None,
    },
];

/// Look up the romaji CSS slug for an annotation `canonical` keyword.
/// Returns `None` for an unknown keyword (callers fall back to a
/// neutral slug).
#[must_use]
pub fn roman_slug(canonical: &str) -> Option<&'static str> {
    RENDER_SLUGS
        .iter()
        .find(|e| e.canonical == canonical)
        .map(|e| e.roman)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_family_wire_tags_complete_and_distinct() {
        assert_eq!(
            SlugFamily::ALL.len(),
            12,
            "every SlugFamily variant listed in ALL"
        );
        let mut tags: Vec<&str> = SlugFamily::ALL.iter().map(|&f| f.as_wire_tag()).collect();
        for t in &tags {
            assert!(!t.is_empty(), "empty wire tag");
            assert_ne!(*t, "unknown", "wire tag must not be a fallback");
        }
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total, "duplicate wire tag in SlugFamily::ALL");
    }

    /// Minimal Hepburn romaniser (long vowel お／う段 + う dropped,
    /// matching the slug convention 改丁→`kaicho`). Test-only — the
    /// production slugs are hand-written in [`RENDER_SLUGS`]; this
    /// re-derives them from `reading` to catch a misspelling.
    #[allow(
        clippy::too_many_lines,
        reason = "exhaustive kana→romaji morae tables read clearest inline"
    )]
    fn hepburn(kana: &str) -> String {
        const TWO: &[(&str, &str)] = &[
            ("きゃ", "kya"),
            ("きゅ", "kyu"),
            ("きょ", "kyo"),
            ("しゃ", "sha"),
            ("しゅ", "shu"),
            ("しょ", "sho"),
            ("ちゃ", "cha"),
            ("ちゅ", "chu"),
            ("ちょ", "cho"),
            ("にゃ", "nya"),
            ("にゅ", "nyu"),
            ("にょ", "nyo"),
            ("ひゃ", "hya"),
            ("ひゅ", "hyu"),
            ("ひょ", "hyo"),
            ("みゃ", "mya"),
            ("みゅ", "myu"),
            ("みょ", "myo"),
            ("りゃ", "rya"),
            ("りゅ", "ryu"),
            ("りょ", "ryo"),
            ("ぎゃ", "gya"),
            ("ぎゅ", "gyu"),
            ("ぎょ", "gyo"),
            ("じゃ", "ja"),
            ("じゅ", "ju"),
            ("じょ", "jo"),
            ("びゃ", "bya"),
            ("びゅ", "byu"),
            ("びょ", "byo"),
            ("ぴゃ", "pya"),
            ("ぴゅ", "pyu"),
            ("ぴょ", "pyo"),
        ];
        const ONE: &[(&str, &str)] = &[
            ("あ", "a"),
            ("い", "i"),
            ("う", "u"),
            ("え", "e"),
            ("お", "o"),
            ("か", "ka"),
            ("き", "ki"),
            ("く", "ku"),
            ("け", "ke"),
            ("こ", "ko"),
            ("が", "ga"),
            ("ぎ", "gi"),
            ("ぐ", "gu"),
            ("げ", "ge"),
            ("ご", "go"),
            ("さ", "sa"),
            ("し", "shi"),
            ("す", "su"),
            ("せ", "se"),
            ("そ", "so"),
            ("ざ", "za"),
            ("じ", "ji"),
            ("ず", "zu"),
            ("ぜ", "ze"),
            ("ぞ", "zo"),
            ("た", "ta"),
            ("ち", "chi"),
            ("つ", "tsu"),
            ("て", "te"),
            ("と", "to"),
            ("だ", "da"),
            ("ぢ", "ji"),
            ("づ", "zu"),
            ("で", "de"),
            ("ど", "do"),
            ("な", "na"),
            ("に", "ni"),
            ("ぬ", "nu"),
            ("ね", "ne"),
            ("の", "no"),
            ("は", "ha"),
            ("ひ", "hi"),
            ("ふ", "fu"),
            ("へ", "he"),
            ("ほ", "ho"),
            ("ば", "ba"),
            ("び", "bi"),
            ("ぶ", "bu"),
            ("べ", "be"),
            ("ぼ", "bo"),
            ("ぱ", "pa"),
            ("ぴ", "pi"),
            ("ぷ", "pu"),
            ("ぺ", "pe"),
            ("ぽ", "po"),
            ("ま", "ma"),
            ("み", "mi"),
            ("む", "mu"),
            ("め", "me"),
            ("も", "mo"),
            ("や", "ya"),
            ("ゆ", "yu"),
            ("よ", "yo"),
            ("ら", "ra"),
            ("り", "ri"),
            ("る", "ru"),
            ("れ", "re"),
            ("ろ", "ro"),
            ("わ", "wa"),
            ("を", "o"),
            ("ん", "n"),
        ];
        let mut out = String::new();
        let mut rest = kana;
        'outer: while !rest.is_empty() {
            // Long-vowel う after an o/u sound collapses (ちょう→cho).
            if let Some(r) = rest.strip_prefix('う')
                && (out.ends_with('o') || out.ends_with('u'))
            {
                rest = r;
                continue;
            }
            for (k, v) in TWO {
                if let Some(r) = rest.strip_prefix(k) {
                    out.push_str(v);
                    rest = r;
                    continue 'outer;
                }
            }
            for (k, v) in ONE {
                if let Some(r) = rest.strip_prefix(k) {
                    out.push_str(v);
                    rest = r;
                    continue 'outer;
                }
            }
            // Unknown kana — surface it so the test fails loudly.
            return format!("{out}?{rest}");
        }
        out
    }

    #[test]
    fn render_slugs_are_kebab_ascii() {
        for e in RENDER_SLUGS {
            assert!(
                !e.roman.is_empty()
                    && e.roman
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-' || b.is_ascii_digit()),
                "non-kebab render slug: {:?}",
                e.roman
            );
        }
    }

    #[test]
    fn render_slugs_are_unique() {
        let mut seen: Vec<&'static str> = Vec::with_capacity(RENDER_SLUGS.len());
        for e in RENDER_SLUGS {
            assert!(
                !seen.contains(&e.roman),
                "duplicate render slug: {}",
                e.roman
            );
            seen.push(e.roman);
        }
    }

    #[test]
    fn render_slug_readings_are_hiragana() {
        for e in RENDER_SLUGS {
            if let Some(r) = e.reading {
                assert!(
                    r.chars().all(|c| ('\u{3040}'..='\u{309F}').contains(&c)),
                    "reading not hiragana: {r} ({})",
                    e.canonical
                );
            }
        }
    }

    #[test]
    fn render_slug_matches_reading() {
        for e in RENDER_SLUGS {
            let Some(reading) = e.reading else { continue };
            let h = hepburn(reading);
            let ok = e.roman == h
                || e.roman
                    .strip_prefix(&h)
                    .is_some_and(|rest| rest.starts_with('-'));
            assert!(
                ok,
                "render slug {:?} (canonical {}) inconsistent with reading {reading} → hepburn {h}",
                e.roman, e.canonical
            );
        }
    }

    #[test]
    fn roman_slug_looks_up_and_misses() {
        assert_eq!(roman_slug("行右小書き"), Some("kogaki-right"));
        assert_eq!(roman_slug("白ゴマ傍点"), Some("shirogoma"));
        assert_eq!(roman_slug("改丁"), Some("kaicho"));
        assert_eq!(roman_slug("nonsense"), None);
    }

    #[test]
    fn slugs_table_is_non_empty() {
        assert!(!SLUGS.is_empty());
    }

    #[test]
    fn slugs_have_unique_canonical_strings() {
        let mut seen: Vec<&'static str> = Vec::with_capacity(SLUGS.len());
        for entry in SLUGS {
            assert!(
                !seen.contains(&entry.canonical),
                "duplicate canonical: {}",
                entry.canonical
            );
            seen.push(entry.canonical);
        }
    }

    #[test]
    fn every_canonical_is_self_canonical() {
        for entry in SLUGS {
            let resolved = canonicalise_slug(entry.canonical)
                .unwrap_or_else(|| panic!("canonical {} did not resolve", entry.canonical));
            assert_eq!(resolved, entry.canonical);
        }
    }

    #[test]
    fn known_hiragana_variants_resolve_to_canonical() {
        assert_eq!(canonicalise_slug("ぼうてん"), Some("傍点"));
        assert_eq!(canonicalise_slug("にぼうてん"), Some("傍点"));
        assert_eq!(canonicalise_slug("しろまるぼうてん"), Some("白丸傍点"));
        assert_eq!(canonicalise_slug("ここからじさげ"), Some("ここから字下げ"));
    }

    #[test]
    fn unknown_input_returns_none() {
        assert_eq!(canonicalise_slug("nonsense"), None);
        assert_eq!(canonicalise_slug(""), None);
    }

    #[test]
    fn paired_slugs_reference_existing_partner() {
        for entry in SLUGS {
            if let Some(partner) = entry.partner {
                let found = SLUGS.iter().any(|e| e.canonical == partner);
                assert!(
                    found,
                    "partner {partner} not in SLUGS for {}",
                    entry.canonical
                );
            }
        }
    }

    #[test]
    fn block_container_open_pairs_with_close() {
        // Every BlockContainerOpen entry must point at a partner whose
        // family is BlockContainerClose, and vice versa.
        for entry in SLUGS {
            match entry.family {
                SlugFamily::BlockContainerOpen => {
                    let partner_canonical = entry
                        .partner
                        .unwrap_or_else(|| panic!("open {} has no partner", entry.canonical));
                    let partner = SLUGS
                        .iter()
                        .find(|e| e.canonical == partner_canonical)
                        .expect("partner exists");
                    assert!(matches!(
                        partner.family,
                        SlugFamily::BlockContainerClose | SlugFamily::Framed | SlugFamily::Warichu
                    ));
                }
                SlugFamily::BlockContainerClose => {
                    let partner_canonical = entry
                        .partner
                        .unwrap_or_else(|| panic!("close {} has no partner", entry.canonical));
                    let partner = SLUGS
                        .iter()
                        .find(|e| e.canonical == partner_canonical)
                        .expect("partner exists");
                    assert!(matches!(
                        partner.family,
                        SlugFamily::BlockContainerOpen | SlugFamily::Framed | SlugFamily::Warichu
                    ));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn accepts_param_aligns_with_brace_in_canonical() {
        // Every entry whose canonical contains `{` must have
        // accepts_param == true, and vice versa.
        for entry in SLUGS {
            let has_brace = entry.canonical.contains('{');
            assert_eq!(
                entry.accepts_param, has_brace,
                "accepts_param/brace mismatch on {}",
                entry.canonical
            );
        }
    }

    #[test]
    fn variant_table_resolves_to_strings_in_slugs() {
        for &(variant, canonical) in VARIANTS {
            assert!(
                SLUGS.iter().any(|e| e.canonical == canonical),
                "variant {variant} maps to unknown canonical {canonical}"
            );
        }
    }
}
