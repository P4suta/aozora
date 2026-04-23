//! Property-based tests for ruby recognition, exercised end-to-end through
//! the lexer + `post_process` pipeline (`afm_parser::parse`).
//!
//! Targets the core round-trip invariant from
//! <https://www.aozora.gr.jp/annotation/ruby.html>: explicit `｜BASE《READING》`
//! spans always round-trip the base and reading literals and set
//! `delim_explicit = true`. Implicit-delimiter forms are covered by the
//! lexer's unit tests in `afm_lexer::phase3_classify`; the proptest here
//! stays scoped to explicit form so the corpus of random inputs is
//! unambiguous.

use afm_parser::{Options, parse};
use afm_syntax::AozoraNode;
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};
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

/// Walk `root` and return the first `AozoraNode::Ruby` encountered, or
/// `None` if the tree contains no ruby. Test-only helper.
fn first_ruby<'a>(root: &'a AstNode<'a>) -> Option<afm_syntax::Ruby> {
    for n in root.descendants() {
        if let NodeValue::Aozora(ref boxed) = n.data.borrow().value
            && let AozoraNode::Ruby(ref r) = **boxed
        {
            return Some(r.clone());
        }
    }
    None
}

proptest! {
    /// Explicit delimiter form always round-trips base/reading exactly
    /// through the full parse pipeline.
    #[test]
    fn explicit_ruby_round_trips(base in kanji_strategy(5), reading in hiragana_strategy(10)) {
        let input = format!("｜{base}《{reading}》");
        let arena = Arena::new();
        let opts = Options::afm_default();
        let root = parse(&arena, &input, &opts);
        let ruby = first_ruby(root).expect("explicit form must parse to a Ruby node");
        prop_assert_eq!(ruby.base.as_plain(), Some(base.as_str()));
        prop_assert_eq!(ruby.reading.as_plain(), Some(reading.as_str()));
        prop_assert!(ruby.delim_explicit);
    }
}
