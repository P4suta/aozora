//! Differential property tests: the LSP `textDocument/formatting`
//! handler (`format_edits`) and the `aozora-fmt` CLI (`format_source`)
//! are documented to produce a **byte-identical** canonical form. An
//! editor's "Format Document" must therefore never disagree with
//! `aozora-fmt --write`. `tests/guardian.rs` pins this on a fixed corpus;
//! here we hold it across a broad, aozora-notation-biased input space so a
//! regression in *either* path surfaces regardless of which one drifts.

use aozora_fmt::format_source;
use aozora_lsp::internals::format_edits;
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;

/// A string biased toward real aozora content, widened well past the
/// position/edit strategy in `property_invariants.rs`: explicit and
/// implicit ruby, double ruby, tortoise-bracket accents, bouten slugs,
/// gaiji with a JIS mencode, mixed/oddball line endings, a PUA sentinel,
/// and a BOM. Concatenating random fragments produces both canonical and
/// non-canonical documents, so both arms of `format_edits` are exercised.
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
        "日本《にほん》",     // implicit ruby (non-canonical → gets a ｜)
        "｜青空《あおぞら》", // explicit ruby (canonical)
        "《《重要》》",       // double ruby
        "〔Crevez chiens〕",  // tortoise-bracket accent decomposition
        "「鉤括弧」",
        "※［＃「弓＋鳥」、第3水準1-2-3］", // gaiji with mencode
        "［＃改ページ］",
        "［＃ぼうてん］", // non-canonical slug
        "彼［＃「彼」に傍点］",
        "😀",       // surrogate pair
        "\u{E001}", // PUA sentinel collision
        "\u{feff}", // BOM
    ];
    proptest_vec(select(fragments), 0usize..14usize).prop_map(|frags| frags.concat())
}

proptest! {
    /// The LSP `formatting` handler and `aozora-fmt` must agree on the
    /// canonical form for every input: zero edits when already canonical,
    /// otherwise a single full-document replace whose `new_text` is exactly
    /// `aozora-fmt`'s output.
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
                "the LSP edit must rewrite to aozora-fmt's canonical form",
            );
        }
    }

    /// Canonicalisation is a fixed point: the canonical form formats to
    /// itself, so the LSP returns zero edits for it. This is the
    /// idempotency contract every consumer (formatter, LSP, CI gate)
    /// relies on, held across the whole input space.
    #[test]
    fn canonical_form_is_a_format_edits_fixed_point(text in aozora_text_strategy()) {
        let canonical = format_source(&text);
        prop_assert!(
            format_edits(&canonical).is_empty(),
            "the canonical form must be a fixed point of format_edits",
        );
    }
}
