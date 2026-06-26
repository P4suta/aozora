//! Differential gate for the #237 owned-AST serializer (P0.2a).
//!
//! Proves the owned representation reproduces Aozora source **byte-for-byte**:
//! for every input, the owned path
//! `serialize_owned(tree.lex_output().to_owned())` must equal the borrowed
//! authority `tree.to_source()`. The two share the same normalized text and
//! position-isomorphic registry (the converter copies `normalized` verbatim),
//! so byte-equality is exactly the proof that every `emit_*_owned` reproduces
//! its borrowed twin and every `StrId`/range resolve recovers the original
//! `&str`.
//!
//! Coverage: a curated set with at least one of every node kind, every
//! `crates/aozora-conformance/fixtures/render/**` source, and — when
//! `AOZORA_CORPUS_ROOT` is set — every corpus document (skipped cleanly when
//! unset, as it is in CI; the curated + conformance inputs carry the gate).

use std::borrow::Cow;

use aozora::Document;
use aozora::render::serialize_owned;
use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_encoding::decode_auto;

/// Assert the owned serializer reproduces the borrowed `to_source` byte-exactly.
fn assert_owned_matches(src: &str) {
    let doc = Document::new(src);
    let tree = doc.parse();
    let owned = tree.lex_output().to_owned();
    assert_eq!(
        serialize_owned(&owned),
        tree.to_source(),
        "owned serializer diverged from borrowed for {src:?}"
    );
}

/// At least one input per node kind / forward-reference form, reusing the exact
/// strings pinned in `aozora-render`'s `serialize` unit tests so the gate is
/// grounded in known fixed points.
const CURATED: &[&str] = &[
    // plain
    "hello world",
    // ruby — bare, bar-kept (3 reasons), left-side
    "｜青梅《おうめ》",
    "頃｜青梅《おうめ》",
    "｜お目《おめ》",
    "｜｜青空《あおぞら》",
    "再読［＃「再読」の左に「さい」のルビ］",
    // margin note — 注記 + 傍記
    "底本「青空」［＃「青空」の左に「注記」の注記］",
    "資本主義の一般的危機［＃「危機」に「×」の傍記］",
    // bouten leaf — default, named kinds, 左に, segmented (、-split)
    "可哀想［＃「可哀想」に傍点］",
    "X［＃「X」に二重丸傍点］",
    "X［＃「X」に傍線］",
    "X［＃「X」の左に傍点］",
    "甲乙［＃「甲」「乙」に傍点］",
    // forward emphasis — bold/italic/tcy/script/fontsize±/caption
    "重要［＃「重要」は太字］",
    "X［＃「X」は斜体］",
    "12［＃「12」は縦中横］",
    "X［＃「X」は行右小書き］",
    "X［＃「X」は行左小書き］",
    "X［＃「X」は3段階大きな文字］",
    "X［＃「X」は2段階小さな文字］",
    "A［＃「A」はキャプション］",
    // gaiji — simple, composed (carries own 「」), standalone (#122)
    "※［＃「○○」、第3水準1-85-54］",
    "※［＃「あ」の「い」に代えて「う」、1-2-3］",
    "※［＃「木＋吶のつくり」、第3水準1-85-54］",
    // kaeriten
    "一二［＃レ］",
    // heading leaf — level × style
    "見出し\n［＃「見出し」は大見出し］",
    "見出し\n［＃「見出し」は中見出し］",
    "見出し\n［＃「見出し」は窓中見出し］",
    "見出し\n［＃「見出し」は同行小見出し］",
    // heading hint — unpromoted (level variants)
    "本文の途中に見出しがある。\n［＃「見出し」は大見出し］",
    "長い前置きの文章があって行頭ではない［＃「見出し」は中見出し］",
    // angle quote — plain + gaiji segment
    "≪重要≫",
    "≪※［＃「○」、第3水準1-85-54］≫",
    // illustration — keyword, dimensions
    "［＃挿絵（fig.png）入る］",
    "［＃挿絵（fig.png、横480×縦640）入る］",
    // line leaves — indent/align-end/center(page+line)/framed
    "［＃2字下げ］",
    "［＃字下げ］",
    "［＃地付き］",
    "［＃地から2字上げ］",
    "［＃ページの左右中央］",
    "［＃中央揃え］",
    "［＃罫囲み］",
    // section/page break, body-end, forced-break
    "［＃改ページ］",
    "［＃改丁］",
    "［＃改段］",
    "［＃改見開き］",
    "本編［＃本文終わり］",
    "行頭［＃改行］行末",
    // unknown directive — raw passthrough
    "［＃字下げ］",
    // containers — open/close sentinels (emit_container_open/close reuse)
    "［＃ここから字下げ］\nA\n［＃ここで字下げ終わり］",
    "［＃ここから2字下げ］\nA\n［＃ここで字下げ終わり］",
    "［＃ここから3字下げ、1行20字組みで］\nA\n［＃ここで字下げ、20字組み終わり］",
    "［＃ここから太字］\nA\n［＃ここで太字終わり］",
    "［＃ここから斜体］\nA\n［＃ここで斜体終わり］",
    "［＃ここから大見出し］\nA\n［＃ここで大見出し終わり］",
    "［＃窓中見出し］A［＃窓中見出し終わり］",
    "［＃ここから2段組み］\nA\n［＃ここで段組み終わり］",
    "［＃ここから表］\nA\n［＃ここで表終わり］",
    "［＃ここから横組み］\nA\n［＃ここで横組み終わり］",
    "［＃ここから地付き］\nA\n［＃ここで地付き終わり］",
    "［＃ここから20字詰め］\nA\n［＃ここで字詰め終わり］",
    "［＃ここから3段階大きな文字］\nA\n［＃ここで大きな文字終わり］",
    // bare inline ranges that fold to forward leaves (S4/S5/S6)
    "［＃傍点］A［＃傍点終わり］",
    "本文［＃太字］註［＃太字終わり］。",
    "12［＃縦中横］34［＃縦中横終わり］",
    // mixed multi-construct document + decorative-rule isolate post-pass
    concat!(
        "冒頭の文。\n",
        "｜青梅《おうめ》が見える。\n",
        "可哀想［＃「可哀想」に傍点］だ。\n",
        "本文［＃太字］強調［＃太字終わり］。\n",
        "12［＃「12」は縦中横］時。\n",
        "≪引用≫もある。\n",
        "［＃ここから2字下げ］\n字下げ本文\n［＃ここで字下げ終わり］\n",
        "［＃改ページ］\n",
        "末尾。",
    ),
    "段落の文\n――――――――――――\n｜青梅《おうめ》の続き\n",
];

#[test]
fn owned_serialize_matches_borrowed_on_curated_inputs() {
    for src in CURATED {
        assert_owned_matches(src);
    }
}

#[test]
fn owned_serialize_matches_borrowed_on_render_fixtures() {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "render fixtures must be present");
    for f in fixtures {
        assert_owned_matches(&f.source);
    }
}

#[test]
fn owned_serialize_matches_borrowed_on_corpus() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping owned-serialize corpus gate");
        return;
    };
    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(utf8): Result<Cow<'_, str>, _> = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };
        assert_owned_matches(&utf8);
    }
}
