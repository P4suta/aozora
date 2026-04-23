//! Isolation test for forward-reference bouten handling.
//!
//! Tier A on the full 罪と罰 fixture was leaking ［＃ at
//! `可哀想［＃「可哀想」に傍点］` — this integration test pins that exact pattern
//! so a regression surfaces immediately instead of only when the full novel is
//! exercised.

use afm_parser::test_support::assert_no_bare;

#[test]
fn forward_reference_bouten_source_span_is_consumed() {
    // Core Tier-A invariant: the ［＃…］ bracket must not leak into the HTML
    // regardless of whether the scanner promotes to Bouten (C3, preceding
    // contains the target) or degrades to Annotation{Unknown} (target not
    // found in preceding). The specific downstream rendering is covered by
    // html.rs tests — this regression suite only pins the span-consumption
    // contract.
    let src = "可哀想［＃「可哀想」に傍点］という気";
    let html = afm_parser::html::render_to_string(src);
    assert_no_bare(&html, "［＃");
}

#[test]
fn forward_reference_bouten_survives_long_paragraph_context() {
    // Exact snippet around the leak 罪と罰 Tier A flags. The critical character
    // class in the prefix is the opening curly quote 「 U+300C, which shares a
    // UTF-8 lead byte (0xE3) with 《 — if `find_aozora_trigger_offset` ever
    // matched 「, the scanner would stop inside an unrelated opening-quote span
    // and the subsequent ［＃ would never get classified.
    let src = "「そう、妹さんの心の中に可哀想［＃「可哀想」に傍点］という気が起こる";
    let html = afm_parser::html::render_to_string(src);
    assert_no_bare(&html, "［＃");
}

#[test]
fn curly_quote_followed_by_bracket_annotation() {
    // Minimal repro: the 「X」 quote form appears in many contexts; the adapter
    // must not confuse 「 with a trigger character.
    let src = "「X」［＃「Y」に傍点］";
    let html = afm_parser::html::render_to_string(src);
    assert_no_bare(&html, "［＃");
}
