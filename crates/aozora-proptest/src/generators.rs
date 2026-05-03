//! Proptest generator strategies for aozora property-based tests.
//!
//! These strategies are intentionally *stratified*: each one targets a
//! specific shape of input that exercises a particular class of
//! invariants. Tests opt into the strategies they need rather than a
//! single monolithic generator, so shrinkers can home in on the
//! offending shape without wandering into unrelated regions of the
//! input space.
//!
//! # Shapes
//!
//! * [`kanji_fragment`] / [`hiragana_fragment`] — single-script CJK
//!   strings for ruby round-trip properties.
//! * [`aozora_fragment`] — mixed plain text + Aozora trigger glyphs,
//!   the workhorse strategy for "parse doesn't crash and doesn't leak"
//!   properties. Includes long `-`/`=`/`_` decorative rule rows so
//!   Tier H (setext-vs-decorative-rule) fires naturally.
//! * [`pathological_aozora`] — unbalanced bracket shapes, runaway
//!   delimiters, adjacent paired-container opens without closes. Drives
//!   the "parse is total" and "malformed input never panics" properties.
//! * [`commonmark_adversarial`] — adversarial CommonMark constructs
//!   (deeply nested blockquotes in lists in headings, backslash
//!   escapes, tight/loose lists) for Aozora × CommonMark interaction
//!   tests.
//! * [`xss_payload`] — hand-curated dangerous payloads for the
//!   "no `<script>` leaks to rendered HTML" property.
//! * [`unicode_adversarial`] — combining marks, RTL overrides, private
//!   use area codepoints (including the PUA sentinels U+E001–U+E004 the
//!   lexer reserves for itself), full-width bracket variants.
//! * [`sjis_bytes`] — arbitrary `Vec<u8>` for exercising
//!   `aozora_encoding::decode_sjis` error paths; most draws are random
//!   bytes so the failure modes of interest all get exercised.

use proptest::prelude::*;

/// Generate a kanji-only string of 1 to `max_len` codepoints from the
/// CJK Unified Ideographs block (U+4E00–U+9FFF).
///
/// Used for ruby-base round-trip properties where the base must be a
/// single explicit-delimiter chunk of characters.
///
/// # Panics
///
/// Does not panic in practice: the `0x4E00..=0x9FFF` range contains
/// only valid non-surrogate codepoints, so [`char::from_u32`] is
/// total over the generated values. The `expect` in the body is
/// retained as a defence-in-depth guard against future range changes.
pub fn kanji_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0x4E00_u32..=0x9FFF, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).expect("CJK range is always valid"))
            .collect()
    })
}

/// Generate a hiragana-only string of 1 to `max_len` codepoints from
/// U+3041–U+3096.
///
/// Used for ruby-reading round-trip properties. The upper bound is
/// U+3096 (rather than U+309F) to exclude the hiragana voicing /
/// iteration marks, which the lexer's ruby parser does not currently
/// special-case — keeping the generator conservative here avoids
/// exercising behaviour that is covered by hand-written unit tests
/// instead.
///
/// # Panics
///
/// Does not panic in practice: the `0x3041..=0x3096` range contains
/// only valid non-surrogate codepoints. The `expect` in the body is
/// retained as a defence-in-depth guard.
pub fn hiragana_fragment(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0x3041_u32..=0x3096, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).expect("hiragana range is always valid"))
            .collect()
    })
}

/// Generate a mixed-shape Aozora fragment composed of 0 to `max_atoms`
/// atoms, chosen from a pool of Aozora trigger glyphs and filler.
///
/// The glyphs covered include `｜`, `《`, `》`, `［＃`, `］`, `※`,
/// known annotation bodies (`改ページ`, `改丁`, `漢字`, `かんじ`),
/// ASCII filler, whitespace, line separators, and long `-`/`=`/`_`
/// decorative rule rows.
///
/// This is the workhorse strategy for "parse is total" and "Tier-A
/// canary holds for well-formed inputs" properties. It deliberately
/// emits *unbalanced* bracket shapes so shrinking can surface
/// malformed-input panics; downstream properties that demand
/// well-formedness gate on the lexer's own diagnostics.
///
/// The decorative rule atoms (≥ 10 repeats of `-`/`=`/`_`) are the
/// bait for Tier H — the phase0 sanitize pass must isolate them so
/// CommonMark does not promote the preceding paragraph into a setext
/// heading. Inputs like `prose\n----------\nbody` must stay as three
/// paragraphs + `<hr>`, not `<h2>prose</h2>`.
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
        // Decorative rule rows — 10 to 50 repeats of the three setext
        // / thematic-break chars. Exercised via Tier H. The row is
        // emitted bare so the surrounding atoms decide whether a
        // newline brackets it.
        Just("-".repeat(12)),
        Just("=".repeat(12)),
        Just("_".repeat(12)),
        Just("-".repeat(35)),
        Just("=".repeat(35)),
    ];
    prop::collection::vec(atoms, 0..=max_atoms).prop_map(|pieces| pieces.join(""))
}

/// Generate an adversarial Aozora source that deliberately exercises
/// malformed / pathological shapes.
///
/// Specifically emits:
///
/// * Long runs of the same trigger glyph (`［＃［＃［＃…`) that stack
///   up on the phase2 pair stack without any matching close.
/// * Adjacent paired-container opens (`［＃ここから字下げ］`) without
///   the expected close (`［＃ここで字下げ終わり］`), and vice versa.
/// * Ruby delimiters in permutations that the classifier must reject
///   gracefully.
///
/// `max_depth` caps how many adversarial atoms are concatenated; a
/// typical call uses 4–8. Values much larger than that push the
/// shrinker into the malformed regime too aggressively for fast
/// iteration.
pub fn pathological_aozora(max_depth: usize) -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just("［＃［＃".to_owned()),
        Just("］］".to_owned()),
        Just("《《".to_owned()),
        Just("》》".to_owned()),
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

/// Generate adversarial CommonMark source.
///
/// Covers deeply nested blockquotes inside lists inside headings,
/// tight/loose list toggles, backslash escapes, fenced-code in
/// blockquotes, and HTML passthrough. Drives the "Aozora × CommonMark
/// interaction doesn't leak sentinels" properties.
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

/// Canned XSS payloads covering the shapes a renderer bug could pass
/// through unescaped.
///
/// All of these must produce output where the dangerous substrings are
/// either absent or escaped — never executable.
///
/// Includes straight `<script>` tags, attribute-style event handlers,
/// URL-encoded bypasses (which must *not* be decoded by the pipeline —
/// they become plain text), and full-width analogs of angle brackets
/// (since aozora already deals with full-width `［` for Aozora markup).
pub fn xss_payload() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<script>alert(1)</script>".to_owned()),
        Just("<script src=x>".to_owned()),
        Just("<SCRIPT>alert(1)</SCRIPT>".to_owned()),
        Just("<script xmlns='http://www.w3.org/1999/xhtml'>bad()</script>".to_owned()),
        Just("<img src=x onerror=alert(1)>".to_owned()),
        Just("<svg onload=alert(1)>".to_owned()),
        Just("<body onload=alert(1)>".to_owned()),
        Just("<iframe src=javascript:alert(1)>".to_owned()),
        Just("<a href=\"javascript:alert(1)\">go</a>".to_owned()),
        Just("<a href='javascript:alert(1)'>go</a>".to_owned()),
        Just("<div onmouseover=alert(1)>hover</div>".to_owned()),
        // URL-encoded bypass: must stay as literal characters, not
        // decoded into `<script>`.
        Just("%3Cscript%3Ealert(1)%3C%2Fscript%3E".to_owned()),
        // Full-width angle brackets — look identical to the reader,
        // should not be normalised into ASCII `<` / `>`.
        Just("＜script＞alert(1)＜/script＞".to_owned()),
        // Mixed content with Aozora markup on either side.
        Just("｜漢字《かんじ》<script>x()</script>".to_owned()),
        Just("［＃「<script>」は大見出し］".to_owned()),
        // CDATA wrapper (the parser's raw-HTML passthrough).
        Just("<![CDATA[<script>bad()</script>]]>".to_owned()),
        // HTML comment smuggling.
        Just("<!--<script>x()</script>-->".to_owned()),
        // data: URI.
        Just("<a href=\"data:text/html,<script>x()</script>\">go</a>".to_owned()),
        // Attribute-break bypass.
        Just("<div class=\"\" onclick=\"alert(1)\">".to_owned()),
        // Misnested opening tag.
        Just("<<script>alert(1)</script>".to_owned()),
    ]
}

/// Generate Unicode edge-case strings.
///
/// Covers combining marks stacked on ASCII bases, RTL override runs,
/// private-use-area codepoints (including the PUA sentinels the lexer
/// uses internally), and full-width bracket variants.
///
/// The PUA range U+E000–U+F8FF is intentionally included. Phase 0 of
/// the lexer emits a diagnostic when a source string already contains
/// the PUA sentinels (U+E001–U+E004) but must not panic — this
/// strategy drives that property.
pub fn unicode_adversarial() -> impl Strategy<Value = String> {
    prop_oneof![
        // Combining marks on ASCII bases.
        Just("a\u{0301}\u{0302}\u{0303}e\u{0304}".to_owned()),
        // RTL override + mixed scripts.
        Just("\u{202E}abc\u{202C}def".to_owned()),
        // BOM in the middle of content.
        Just("pre\u{FEFF}post".to_owned()),
        // PUA sentinels the lexer reserves (U+E001..U+E004).
        Just("X\u{E001}Y".to_owned()),
        Just("X\u{E002}Y".to_owned()),
        Just("X\u{E003}Y".to_owned()),
        Just("X\u{E004}Y".to_owned()),
        // Broader PUA range.
        Just("\u{E5FF}\u{F8FF}text".to_owned()),
        // Full-width bracket variants of the Aozora triggers.
        Just("［＃ｂｒａｃｋｅｔ］".to_owned()),
        // Zero-width joiners and non-joiners.
        Just("a\u{200C}b\u{200D}c".to_owned()),
        // Variation selector.
        Just("字\u{FE0F}".to_owned()),
        // CJK full-width forms.
        Just("ＡＢＣ１２３".to_owned()),
        // Supplementary plane (emoji).
        Just("😀🎌🗾".to_owned()),
    ]
}

/// Generate arbitrary byte strings for `aozora_encoding::decode_sjis`'s
/// error path.
///
/// Most draws are random `Vec<u8>`; the strategy deliberately does
/// *not* try to bias toward valid SJIS because the failure modes of
/// interest (decode errors, EOF-in-trail-byte, trailing garbage) all
/// live in the error path.
pub fn sjis_bytes(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..=max_len)
}

/// Generate byte sequences biased toward SJIS lead-byte / trail-byte
/// boundaries.
///
/// SJIS uses two ranges of lead bytes (`0x81..=0x9F` + `0xE0..=0xEF`)
/// followed by trail bytes (`0x40..=0x7E` + `0x80..=0xFC`); a decoder
/// that mishandles the boundary between them (e.g. accepts an
/// out-of-range trail or runs off the end with an unpaired lead) is
/// the most common SJIS bug class.
///
/// The generator emits one of: a valid lead+trail pair, an unpaired
/// lead at end-of-input, an out-of-range trail, or filler ASCII.
/// Inputs are deliberately short (≤ 64 bytes) so the shrinker can
/// pinpoint the offending boundary cleanly.
pub fn sjis_boundary_bytes() -> impl Strategy<Value = Vec<u8>> {
    let atoms = prop_oneof![
        // Valid lead-byte ranges followed by an in-range trail.
        Just(vec![0x81_u8, 0x40]),
        Just(vec![0x9F_u8, 0xFC]),
        Just(vec![0xE0_u8, 0x80]),
        Just(vec![0xEF_u8, 0xFC]),
        // Unpaired lead at end-of-input (decoder must error, not panic).
        Just(vec![0x81_u8]),
        Just(vec![0xE0_u8]),
        // Out-of-range trail after a valid lead.
        Just(vec![0x81_u8, 0x39]),
        Just(vec![0x81_u8, 0xFD]),
        // Plain ASCII filler.
        Just(b"abc".to_vec()),
        Just(b"012".to_vec()),
        // Lead immediately followed by another lead.
        Just(vec![0x81_u8, 0x81_u8, 0x40]),
    ];
    prop::collection::vec(atoms, 0..=16).prop_map(|chunks| chunks.into_iter().flatten().collect())
}

/// Generate deeply nested paired delimiters up to `max_depth` nesting
/// levels.
///
/// `《`/`》` and `［＃`/`］` open/close pairs nested without any
/// content between. Drives the parser's stack-based pairing logic
/// against pathological depth — a regression that would slip past
/// the workhorse `aozora_fragment` generator (which spreads atoms
/// thin) but still corrupt the parser's frame stack.
///
/// `max_depth` is an upper bound; the generator draws a nesting depth
/// in `0..=max_depth` so shrinking can report the smallest failing
/// depth.
pub fn nested_pairs(max_depth: usize) -> impl Strategy<Value = String> {
    (0_usize..=max_depth, 0_usize..=3).prop_map(|(depth, shape)| {
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
