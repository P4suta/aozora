//! Verify formatter robustness and idempotence over generated inputs.
#![cfg(feature = "fmt")]

use aozora::fmt::{format_source, format_source_with};
use aozora::{DirectiveNormalization, SerializeOptions};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;

fn document_strategy() -> impl Strategy<Value = String> {
    let fragments: Vec<&'static str> = vec![
        "",
        "a",
        "abc",
        " ",
        "\t",
        "\n",
        "\r\n",
        "\n\n",
        "あ",
        "本文",
        "漢字並び",
        "｜青空《あおぞら》",
        "青空《あおぞら》",
        "｜",
        "《",
        "》",
        "《》",
        "※［＃「あ」、U+3042］",
        "［＃",
        "］",
        "［＃改ページ］",
        "［＃ここから２字下げ］",
        "「",
        "」",
        "「ほら」",
        "〔cafe〕",
        "〔",
        "〕",
        "《《",
        "》》",
        "《《《《",
        "［＃［＃［＃",
        "｜｜｜",
        "😀",
        "\u{1F600}\u{200D}\u{1F525}",
        "\u{FEFF}",
        "\u{E001}",
        "\u{E002}",
        "\u{E003}",
        "\u{E004}",
        "\u{0}",
        "\u{7f}",
        "\u{a0}",
        "————",
        "※",
        "#",
        "[",
        "]",
        "<",
        ">",
        "|",
        "&",
        "\"",
    ];
    proptest_vec(select(fragments), 0usize..40usize).prop_map(|frags| frags.concat())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn format_source_never_panics(text in document_strategy()) {
        let _formatted = format_source(&text);
    }

    #[test]
    fn format_source_is_idempotent(text in document_strategy()) {
        let once = format_source(&text);
        let twice = format_source(&once);
        prop_assert_eq!(&once, &twice, "second pass changed the output");
    }

    #[test]
    fn format_source_with_fix_is_idempotent(text in document_strategy()) {
        let fix = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
        let once = format_source_with(&text, fix);
        let twice = format_source_with(&once, fix);
        prop_assert_eq!(&once, &twice, "second fix pass changed the output");
    }
}

#[test]
fn deep_nesting_does_not_panic() {
    for depth in [16usize, 256, 1024] {
        let _ruby = format_source(&"《".repeat(depth));
        let _chuki = format_source(&"［＃".repeat(depth));
        let _mixed = format_source(&format!(
            "{}本文{}",
            "｜青空《".repeat(depth),
            "》".repeat(depth),
        ));
    }
}
