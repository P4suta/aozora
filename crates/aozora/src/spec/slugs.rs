use crate::spec::PairKind;

/// Directive category for editor completion entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SlugFamily {
    /// `［＃改ページ］` and other section-level page breaks.
    PageBreak,
    /// Section break such as `［＃改丁］`.
    Section,
    /// Block opener `［＃ここから...］`, paired with [`Self::BlockContainerClose`].
    BlockContainerOpen,
    /// Block closer `［＃ここで...終わり］`.
    BlockContainerClose,
    /// Layout directive for the preceding paragraph.
    LeafAlign,
    /// Emphasis marks on a preceding text span.
    Bouten,
    /// Inline figure reference.
    Illustration,
    /// Framed rule opener or closer.
    Framed,
    /// Inline note opener or closer.
    Warichu,
    /// Horizontal text within vertical writing.
    CombineUpright,
    /// A single kanbun return mark.
    KaeritenSingle,
    /// Compound kanbun return mark.
    KaeritenCompound,
}

impl SlugFamily {
    /// Every variant in declaration order.
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

    /// Stable `camelCase` identifier.
    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
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
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SlugEntry {
    /// Canonical directive body without `［＃` and `］`.
    pub canonical: &'static str,
    /// Directive category.
    pub family: SlugFamily,
    /// Whether the canonical text takes a numeric parameter or illustration path.
    pub accepts_param: bool,
    /// Japanese description for editor completion details.
    pub doc: &'static str,
    /// Canonical partner text for a paired marker, or `None`.
    pub partner: Option<&'static str>,
    /// Delimiter family used to wrap the directive body.
    pub wrapper: PairKind,
}

pub(crate) const SLUGS: &[SlugEntry] = &[
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

const VARIANTS: &[(&str, &str)] = &[
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
    ("かいぺーじ", "改ページ"),
    ("ページかえ", "改ページ"),
    ("かいちょう", "改丁"),
    ("かいだん", "改段"),
    ("かいみひらき", "改見開き"),
    ("ここからじさげ", "ここから字下げ"),
    ("ここでじさげおわり", "ここで字下げ終わり"),
    ("ここからじつき", "ここから地付き"),
    ("ここでじつきおわり", "ここで地付き終わり"),
    ("じつき", "地付き"),
    ("たてちゅうよこ", "縦中横"),
    ("たて中横", "縦中横"),
    ("そうにゅうえ", "挿絵（{path}）入る"),
    ("けいがこみ", "罫囲み"),
    ("けいがこみおわり", "罫囲み終わり"),
    ("わりちゅう", "割り注"),
    ("わりちゅうおわり", "割り注終わり"),
];

#[must_use]
pub(crate) fn canonicalise_slug(input: &str) -> Option<&'static str> {
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

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct RenderSlug {
    pub canonical: &'static str,
    #[cfg_attr(
        all(not(test), not(feature = "json")),
        expect(
            dead_code,
            reason = "reading metadata is consumed by wire generation and catalogue validation"
        )
    )]
    pub reading: Option<&'static str>,
    pub roman: &'static str,
    #[cfg_attr(
        all(not(test), not(feature = "json")),
        expect(
            dead_code,
            reason = "standards metadata is consumed by wire generation and catalogue validation"
        )
    )]
    pub jis: Option<&'static str>,
}

pub(crate) const RENDER_SLUGS: &[RenderSlug] = &[
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
    RenderSlug {
        canonical: "斜体",
        reading: Some("しゃたい"),
        roman: "shatai",
        jis: None,
    },
    RenderSlug {
        canonical: "太字",
        reading: Some("ふとじ"),
        roman: "futoji",
        jis: None,
    },
    RenderSlug {
        canonical: "ゴシック体",
        reading: None,
        roman: "goshikku",
        jis: None,
    },
    RenderSlug {
        canonical: "上付き小文字",
        reading: Some("うわつき"),
        roman: "uwatsuki",
        jis: Some("07.12.01"),
    },
    RenderSlug {
        canonical: "下付き小文字",
        reading: Some("したつき"),
        roman: "shitatsuki",
        jis: Some("07.12.02"),
    },
    RenderSlug {
        canonical: "分数",
        reading: None,
        roman: "bunsu",
        jis: None,
    },
    RenderSlug {
        canonical: "特大文字",
        reading: None,
        roman: "font-extra-large",
        jis: None,
    },
    RenderSlug {
        canonical: "大文字",
        reading: None,
        roman: "font-large",
        jis: None,
    },
    RenderSlug {
        canonical: "中文字",
        reading: None,
        roman: "font-medium",
        jis: None,
    },
    RenderSlug {
        canonical: "小文字",
        reading: None,
        roman: "font-small",
        jis: None,
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
        reading: Some("よこぐみ"),
        roman: "yokogumi",
        jis: None,
    },
    RenderSlug {
        canonical: "キャプション",
        reading: None,
        roman: "caption",
        jis: None,
    },
];

#[must_use]
pub(crate) fn roman_slug(canonical: &str) -> Option<&'static str> {
    RENDER_SLUGS
        .iter()
        .find(|e| e.canonical == canonical)
        .map(|e| e.roman)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::lex;
    use crate::spec::codes;

    #[test]
    fn slug_family_wire_tags_complete_and_distinct() {
        assert_eq!(
            SlugFamily::ALL.len(),
            12,
            "every SlugFamily variant listed in ALL"
        );
        let mut tags: Vec<&str> = SlugFamily::ALL.iter().map(|&f| f.as_json_tag()).collect();
        for t in &tags {
            assert!(!t.is_empty(), "empty wire tag");
            assert_ne!(*t, "unknown", "wire tag must not be a fallback");
        }
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total, "duplicate wire tag in SlugFamily::ALL");
    }

    #[expect(
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
    fn render_slug_jis_clauses_are_numeric() {
        for entry in RENDER_SLUGS {
            if let Some(clause) = entry.jis {
                assert!(
                    clause.split('.').all(|component| !component.is_empty()
                        && component.bytes().all(|b| b.is_ascii_digit())),
                    "invalid JIS clause: {clause}"
                );
            }
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

    fn instantiate(canonical: &str) -> String {
        canonical.replace("{N}", "2").replace("{path}", "fig01.png")
    }

    fn wrap_for_family(family: SlugFamily, body: &str) -> String {
        match family {
            SlugFamily::Bouten => format!("対象［＃「対象」に{body}］"),
            SlugFamily::CombineUpright => format!("対象［＃「対象」は{body}］"),
            _ => format!("［＃{body}］"),
        }
    }

    #[test]
    fn every_canonical_slug_parses_without_residual_marker() {
        for entry in SLUGS {
            let body = instantiate(entry.canonical);
            let source = wrap_for_family(entry.family, &body);
            let out = lex(&source);
            for diag in &out.diagnostics {
                assert!(
                    diag.code() != codes::RESIDUAL_ANNOTATION_MARKER,
                    "canonical slug {} (instantiated: {body}) leaked through as a residual \
                     annotation marker; source = {source}",
                    entry.canonical
                );
            }
        }
    }
}
