//! Compares language-server formatting with [`aozora::fmt::format_source`].

use aozora::fmt::format_source;
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;

use super::formatting::format_edits;

const TEXT_FRAGMENT_COUNT_LIMIT: usize = 14;

fn aozora_text_strategy() -> impl Strategy<Value = String> {
    let fragments: Vec<&'static str> = vec![
        "",
        "a",
        "abc",
        " ",
        "\n",
        "\r\n",
        "\n\n",
        "\r",
        "あ",
        "本文",
        "日本《にほん》",
        "｜青空《あおぞら》",
        "《《重要》》",
        "〔Crevez chiens〕",
        "「鉤括弧」",
        "※［＃「弓＋鳥」、第3水準1-2-3］",
        "［＃改ページ］",
        "［＃ぼうてん］",
        "彼［＃「彼」に傍点］",
        "😀",
        "\u{E001}",
        "\u{feff}",
    ];
    proptest_vec(select(fragments), 0usize..TEXT_FRAGMENT_COUNT_LIMIT)
        .prop_map(|frags| frags.concat())
}

proptest! {
    #[test]
    fn lsp_format_edits_agree_with_aozora_fmt(text in aozora_text_strategy()) {
        let canonical = format_source(&text);
        let edits = format_edits(&text);
        if text == canonical {
            prop_assert!(
                edits.is_empty(),
                "already-canonical input must yield no edits, got {edits:?}",
            );
        } else {
            prop_assert_eq!(edits.len(), 1, "non-canonical input yields one replace edit");
            prop_assert_eq!(
                &edits[0].new_text,
                &canonical,
                "the LSP edit must rewrite to the formatter's canonical form",
            );
        }
    }

    #[test]
    fn canonical_form_is_a_format_edits_fixed_point(text in aozora_text_strategy()) {
        let canonical = format_source(&text);
        prop_assert!(
            format_edits(&canonical).is_empty(),
            "the canonical form must be a fixed point of format_edits",
        );
    }
}
