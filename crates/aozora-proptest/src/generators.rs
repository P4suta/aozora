#![expect(
    clippy::expect_used,
    reason = "the generated branch sets are static and non-empty"
)]

//! Generates source text and encoded bytes for aozora property tests.

use proptest::prelude::*;

const KANJI_START: u32 = 0x4E00;
const KANJI_END: u32 = 0x9FFF;
const HIRAGANA_START: u32 = 0x3041;
const HIRAGANA_END: u32 = 0x3096;
const SHORT_RULE_LENGTH: usize = 12;
const LONG_RULE_LENGTH: usize = 35;
const NAMESPACED_SCRIPT_PAYLOAD: &str =
    "<script xmlns='http://www.w3.org/1999/xhtml'>bad()</script>";
const SJIS_LOW_LEAD_START: u8 = 0x81;
const SJIS_LOW_LEAD_END: u8 = 0x9F;
const SJIS_HIGH_LEAD_START: u8 = 0xE0;
const SJIS_HIGH_LEAD_END: u8 = 0xEF;
const SJIS_LOW_TRAIL_START: u8 = 0x40;
const SJIS_HIGH_TRAIL_START: u8 = 0x80;
const SJIS_TRAIL_END: u8 = 0xFC;
const SJIS_TRAIL_BELOW_RANGE: u8 = 0x39;
const SJIS_TRAIL_ABOVE_RANGE: u8 = 0xFD;
const MAX_BOUNDARY_ATOMS: usize = 16;

/// Generates one to `max_len` kanji characters from the Unicode unified ideograph block.
///
/// # Panics
/// Panics if `max_len` is zero.
pub fn kanji_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(KANJI_START..=KANJI_END, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).expect("CJK range is always valid"))
            .collect()
    })
}

/// Generates one to `max_len` hiragana characters without combining voicing marks.
///
/// # Panics
/// Panics if `max_len` is zero.
pub fn hiragana_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(HIRAGANA_START..=HIRAGANA_END, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).expect("hiragana range is always valid"))
            .collect()
    })
}

/// Generates up to `max_atoms` notation tokens, text fragments, or decorative rules.
/// The output can contain unbalanced delimiters.
pub fn aozora_fragment(max_atoms: usize) -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just("｜".to_owned()),
        Just("《".to_owned()),
        Just("》".to_owned()),
        Just("［＃".to_owned()),
        Just("］".to_owned()),
        Just("※".to_owned()),
        Just("改ページ".to_owned()),
        Just("改丁".to_owned()),
        Just("漢字".to_owned()),
        Just("かんじ".to_owned()),
        Just("ABC".to_owned()),
        Just("1234".to_owned()),
        Just("\n".to_owned()),
        Just("\n\n".to_owned()),
        Just("、".to_owned()),
        Just("。".to_owned()),
        Just(" ".to_owned()),
        Just("-".repeat(SHORT_RULE_LENGTH)),
        Just("=".repeat(SHORT_RULE_LENGTH)),
        Just("_".repeat(SHORT_RULE_LENGTH)),
        Just("-".repeat(LONG_RULE_LENGTH)),
        Just("=".repeat(LONG_RULE_LENGTH)),
    ];
    prop::collection::vec(atoms, 0..=max_atoms).prop_map(|pieces| pieces.join(""))
}

/// Generates up to `max_depth` malformed notation fragments.
pub fn pathological_aozora(max_depth: usize) -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just("［＃［＃".to_owned()),
        Just("］］".to_owned()),
        Just("《《".to_owned()),
        Just("》》".to_owned()),
        Just("≪".to_owned()),
        Just("≫".to_owned()),
        Just("｜｜".to_owned()),
        Just("※［＃".to_owned()),
        Just("［＃ここから字下げ］".to_owned()),
        Just("［＃ここで字下げ終わり］".to_owned()),
        Just("［＃ここから罫囲み］".to_owned()),
        Just("［＃ここで罫囲み終わり］".to_owned()),
        Just("［＃「」は大見出し］".to_owned()),
        Just("［＃「X」に傍点］".to_owned()),
        Just("｜ABC《".to_owned()),
        Just("》DEF".to_owned()),
        Just("［＃改".to_owned()),
        Just("］".to_owned()),
        Just("\n\n".to_owned()),
    ];
    prop::collection::vec(atoms, 0..=max_depth).prop_map(|pieces| pieces.join(""))
}

/// Generates CommonMark constructs for tests of literal source preservation.
pub fn commonmark_adversarial() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("# heading\n\n- item\n  > quote in list\n    1. nested".to_owned()),
        Just("> outer\n> > inner\n> > > deepest\n".to_owned()),
        Just("- loose\n\n- items\n\n- here\n".to_owned()),
        Just("- tight\n- items\n- here\n".to_owned()),
        Just("\\*escaped\\* and \\[not a link\\]\n".to_owned()),
        Just("```rust\nlet x = 1;\n```\n".to_owned()),
        Just("| h1 | h2 |\n| -- | -- |\n| a  | b  |\n".to_owned()),
        Just("[link](url) and ![img](src)\n".to_owned()),
        Just("***\n\nthematic\n\n***\n".to_owned()),
        Just("  <em>inline HTML</em>\n".to_owned()),
        Just("trailing   spaces  \nhard break\n".to_owned()),
    ]
}

/// Generates script injection payloads for escaping tests.
pub fn xss_payload() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<script>alert(1)</script>".to_owned()),
        Just("<script src=x>".to_owned()),
        Just("<SCRIPT>alert(1)</SCRIPT>".to_owned()),
        Just(NAMESPACED_SCRIPT_PAYLOAD.to_owned()),
        Just("<img src=x onerror=alert(1)>".to_owned()),
        Just("<svg onload=alert(1)>".to_owned()),
        Just("<body onload=alert(1)>".to_owned()),
        Just("<iframe src=javascript:alert(1)>".to_owned()),
        Just("<a href=\"javascript:alert(1)\">go</a>".to_owned()),
        Just("<a href='javascript:alert(1)'>go</a>".to_owned()),
        Just("<div onmouseover=alert(1)>hover</div>".to_owned()),
        Just("%3Cscript%3Ealert(1)%3C%2Fscript%3E".to_owned()),
        Just("＜script＞alert(1)＜/script＞".to_owned()),
        Just("｜漢字《かんじ》<script>x()</script>".to_owned()),
        Just("［＃「<script>」は大見出し］".to_owned()),
        Just("<![CDATA[<script>bad()</script>]]>".to_owned()),
        Just("<!--<script>x()</script>-->".to_owned()),
        Just("<a href=\"data:text/html,<script>x()</script>\">go</a>".to_owned()),
        Just("<div class=\"\" onclick=\"alert(1)\">".to_owned()),
        Just("<<script>alert(1)</script>".to_owned()),
    ]
}

/// Generates combining marks, direction controls, private characters, and bracket variants.
pub fn unicode_adversarial() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("a\u{0301}\u{0302}\u{0303}e\u{0304}".to_owned()),
        Just("\u{202E}abc\u{202C}def".to_owned()),
        Just("pre\u{FEFF}post".to_owned()),
        Just("X\u{E001}Y".to_owned()),
        Just("X\u{E002}Y".to_owned()),
        Just("X\u{E003}Y".to_owned()),
        Just("X\u{E004}Y".to_owned()),
        Just("\u{E5FF}\u{F8FF}text".to_owned()),
        Just("［＃ｂｒａｃｋｅｔ］".to_owned()),
        Just("a\u{200C}b\u{200D}c".to_owned()),
        Just("字\u{FE0F}".to_owned()),
        Just("ＡＢＣ１２３".to_owned()),
        Just("😀🎌🗾".to_owned()),
    ]
}

/// Generates up to `max_len` arbitrary bytes for `aozora::decode_sjis`.
pub fn sjis_bytes(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..=max_len)
}

/// Generates short byte sequences at `Shift_JIS` lead and trail boundaries.
pub fn sjis_boundary_bytes() -> impl Strategy<Value = Vec<u8>> {
    let atoms = prop_oneof![
        Just(vec![SJIS_LOW_LEAD_START, SJIS_LOW_TRAIL_START]),
        Just(vec![SJIS_LOW_LEAD_END, SJIS_TRAIL_END]),
        Just(vec![SJIS_HIGH_LEAD_START, SJIS_HIGH_TRAIL_START]),
        Just(vec![SJIS_HIGH_LEAD_END, SJIS_TRAIL_END]),
        Just(vec![SJIS_LOW_LEAD_START]),
        Just(vec![SJIS_HIGH_LEAD_START]),
        Just(vec![SJIS_LOW_LEAD_START, SJIS_TRAIL_BELOW_RANGE]),
        Just(vec![SJIS_LOW_LEAD_START, SJIS_TRAIL_ABOVE_RANGE]),
        Just(b"abc".to_vec()),
        Just(b"012".to_vec()),
        Just(vec![
            SJIS_LOW_LEAD_START,
            SJIS_LOW_LEAD_START,
            SJIS_LOW_TRAIL_START
        ]),
    ];
    prop::collection::vec(atoms, 0..=MAX_BOUNDARY_ATOMS)
        .prop_map(|chunks| chunks.into_iter().flatten().collect())
}

/// Generates nested delimiters up to `max_depth`, including mismatched pairs.
pub fn nested_pairs(max_depth: usize) -> impl Strategy<Value = String> {
    const LAST_DELIMITER_SHAPE: usize = 3;
    (0_usize..=max_depth, 0_usize..=LAST_DELIMITER_SHAPE).prop_map(|(depth, shape)| {
        let (open, close) = match shape {
            0 => ("《", "》"),
            1 => ("［＃", "］"),
            2 => ("｜", "》"),
            _ => ("〔", "〕"),
        };
        let mut s = String::with_capacity((open.len() + close.len()) * depth);
        for _ in 0..depth {
            s.push_str(open);
        }
        for _ in 0..depth {
            s.push_str(close);
        }
        s
    })
}
