//! Fuzz-style robustness properties for [`aozora::fmt::format_source`] — the
//! parse → serialize round trip the formatter (and the LSP formatting handler)
//! run on every document.
//!
//! These are invariants over a large input space rather than hand-picked
//! cases. They treat the parser as a semi-trusted boundary and pin the two
//! properties that matter for a tool that rewrites files in place:
//!
//! 1. **No panic** on any input — a panic in the library is a crash for every
//!    embedder (CLI exit 101; a dropped task in the LSP).
//! 2. **Idempotence** — `format(format(x)) == format(x)`. The CLI's `--write`
//!    mode relies on this fixed point; a violation means the formatter would
//!    rewrite its own canonical output.
#![cfg(feature = "fmt")]

use aozora::fmt::{format_source, format_source_with};
use aozora::render::{DirectiveNormalization, SerializeOptions};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;

/// Strategy biased toward what the formatter actually sees — aozora notation
/// fragments — plus deliberately nasty bytes (PUA sentinels, BOM, control
/// characters, lone delimiters, small nested groups) that adversarial input
/// would carry.
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

    /// `format_source` must never panic, however malformed the input.
    #[test]
    fn format_source_never_panics(text in document_strategy()) {
        let _formatted = format_source(&text);
    }

    /// `format_source` is a fixed point after a single pass:
    /// `format(format(x)) == format(x)`. `--write` depends on this, so a
    /// counter-example is a release blocker.
    #[test]
    fn format_source_is_idempotent(text in document_strategy()) {
        let once = format_source(&text);
        let twice = format_source(&once);
        prop_assert_eq!(&once, &twice, "second pass changed the output");
    }

    /// The `--fix` autofix path is *also* a fixed point:
    /// `format_with(format_with(x)) == format_with(x)`. `--write` combined with
    /// `--fix` runs the `write_back` idempotency guard against this, so a
    /// counter-example would refuse to rewrite (or corrupt) files.
    #[test]
    fn format_source_with_fix_is_idempotent(text in document_strategy()) {
        let fix = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
        let once = format_source_with(&text, fix);
        let twice = format_source_with(&once, fix);
        prop_assert_eq!(&once, &twice, "second fix pass changed the output");
    }
}

/// Deeply nested delimiters are the classic recursive-parser stressor. Bounded
/// to a depth a healthy stack absorbs; a future parser change that regresses
/// here surfaces as a crash in CI.
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
