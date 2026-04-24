//! Property test — Tier C: promoted headings carry only legitimate content.
//!
//! `［＃「X」は大見出し／中見出し／小見出し］` promotes a paragraph
//! into an `<h1>` / `<h2>` / `<h3>`. Commit 7f5463a fixed a latent bug
//! where the post-process promotion failed to strip sibling
//! indent markers from the heading's body:
//! `［＃２字下げ］第一篇［＃「第一篇」は大見出し］` was rendering as
//! `<h1><span class="afm-indent afm-indent-2"></span>第一篇</h1>` —
//! the indent leaked into the heading body. A single regression test
//! in `tests/heading_promotion.rs` guards the specific shape. This
//! property test generalises: *any* random composition of indent
//! markers and heading hints must still produce headings whose bodies
//! carry only the target text (no `afm-indent`, `afm-container-indent`,
//! or `afm-annotation` tokens).
//!
//! # Generator strategy
//!
//! The strategy builds a heading-biased Aozora fragment by
//! concatenating:
//!
//! 1. An indent / align decorator (`［＃N字下げ］`, `［＃ここから字下げ］`,
//!    `［＃地付き］`) chosen from a short list.
//! 2. A target literal (1–5 kanji codepoints).
//! 3. A heading hint (`［＃「target」は大見出し］` / `…中見出し］` /
//!    `…小見出し］`) referencing the target.
//! 4. Optional trailing body text.
//!
//! The lexer's forward-reference classifier requires the target
//! literal to appear before the hint, which the generator provides by
//! construction — this keeps the proptest exercising the promotion
//! path rather than the "unknown annotation" fallback.
//!
//! The generator deliberately over-samples the indent-followed-by-heading
//! shape that commit 7f5463a's bug relied on.

use afm_parser::html::render_to_string;
use afm_parser::test_support::check_heading_integrity;
use afm_test_utils::config::default_config;
use afm_test_utils::generators::kanji_fragment;
use proptest::prelude::*;

/// Generate an indent / alignment decorator as a single atom.
fn indent_atom() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("［＃１字下げ］".to_owned()),
        Just("［＃２字下げ］".to_owned()),
        Just("［＃３字下げ］".to_owned()),
        Just("［＃ここから２字下げ］".to_owned()),
        Just("［＃ここで字下げ終わり］".to_owned()),
        Just("［＃地付き］".to_owned()),
    ]
}

/// Generate a heading-hint suffix (`大`/`中`/`小`) that will wrap the
/// given target.
fn heading_kind() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("大"), Just("中"), Just("小")]
}

/// Compose a heading-biased source: `[decorator][target][hint][trailing]`.
/// The generator picks the decorator and heading kind independently so
/// every combination of shape × kind gets exercised.
fn heading_biased_src() -> impl Strategy<Value = String> {
    (
        indent_atom(),
        kanji_fragment(5),
        heading_kind(),
        kanji_fragment(5),
    )
        .prop_map(|(deco, target, kind, trailing)| {
            format!("{deco}{target}［＃「{target}」は{kind}見出し］\n\n{trailing}")
        })
}

proptest! {
    #![proptest_config(default_config())]

    /// For every heading-biased input, the rendered `<h1>`/`<h2>`/`<h3>`
    /// must carry only the target text — no `afm-indent` /
    /// `afm-container-indent` / `afm-annotation` class should appear
    /// inside the heading body.
    #[test]
    fn heading_body_never_carries_forbidden_classes(src in heading_biased_src()) {
        let html = render_to_string(&src);
        check_heading_integrity(&html)
            .unwrap_or_else(|e| panic!("Tier C violated for src={src:?}, html={html}: {e}"));
    }
}
