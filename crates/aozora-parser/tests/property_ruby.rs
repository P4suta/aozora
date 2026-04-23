//! Property-based tests for the ruby parser. Targets round-trip invariants and the
//! `｜`-required boundary rule from `https://www.aozora.gr.jp/annotation/ruby.html`.

use afm_parser::aozora::ruby;
use proptest::prelude::*;

fn kanji_strategy(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0x4E00_u32..=0x9FFF, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).unwrap())
            .collect()
    })
}

fn hiragana_strategy(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0x3041_u32..=0x3096, 1..=max_len).prop_map(|codepoints| {
        codepoints
            .into_iter()
            .map(|c| char::from_u32(c).unwrap())
            .collect()
    })
}

proptest! {
    /// Explicit delimiter form always round-trips base/reading exactly.
    #[test]
    fn explicit_ruby_round_trips(base in kanji_strategy(5), reading in hiragana_strategy(10)) {
        let input = format!("{base}《{reading}》");
        // The parser lives on the comrak fork path; for M0 we exercise the standalone
        // helper directly to lock in the invariant.
        let (ruby, consumed) =
            ruby::parse(&input, true, "")
                .expect("explicit form must parse");
        prop_assert_eq!(&*ruby.base, base.as_str());
        prop_assert_eq!(&*ruby.reading, reading.as_str());
        prop_assert!(ruby.delim_explicit);
        prop_assert_eq!(consumed, input.len());
    }
}
