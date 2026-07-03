//! Byte-identical golden-HTML gate over a small set of REAL 青空文庫
//! works.
//!
//! The crafted per-family fixtures under `fixtures/render/` isolate one
//! notation construct each. This gate is complementary: it renders a
//! lean, hand-picked set of *whole* public-domain works — the CRLF is
//! normalised to LF and the source is vendored verbatim under
//! `fixtures/works/<slug>/source.txt` — and byte-compares
//! `Document::new(src).parse().to_html()` to the committed
//! `expected.html`. Its job is to catch rendering drift on the
//! notation *combinations* real works exhibit (ruby beside 傍点 beside
//! 縦中横 beside 字下げ …) that the single-construct fixtures cannot see.
//!
//! It is corpus-free: the works are vendored, so the gate reads no
//! `AOZORA_CORPUS_ROOT` and always runs — it lives in the `conformance`
//! CI job next to the spec-vector and reference-grammar checks.
//!
//! The committed golden is the parser's *own* `to_html()` output — a
//! drift-detection baseline, not an independent oracle. After an
//! intentional renderer change, regenerate and review the diff:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate
//! ```
//!
//! Vendored works and the family each was chosen to exercise:
//!
//! | slug                | families exercised                                    |
//! |---------------------|-------------------------------------------------------|
//! | akizuki-genshiroku  | 返り点/訓点 (kaeriten, 296×) + 外字                    |
//! | caldecott-queen     | 挿絵 (illustration, 66× `<img>`)                       |
//! | chiri-kaeru         | 外字 (gaiji, 24×)                                      |
//! | fukuzawa-nikushoku  | 割り注 (warichu) + 左ルビ + 字下げ + 外字             |
//! | miyoshi-nansoshu    | 見出し + 字下げ + 文字サイズ(小) + 外字 + ルビ        |
//! | murayama-ahiru      | 見出し + 字下げ + ルビ (clean children's story)        |
//! | ogawa-koinobori     | near-plain, ルビ-heavy (94×)                           |
//! | orikuchi-matoi      | 傍線 (bosen, 98×) + 外字 + ルビ                        |
//! | orikuchi-sekijin    | 縦中横 + 傍線 + 返り点 + 傍点 + 折り返し字下げ + ルビ  |
//! | potter-peter        | 挿絵 (illustration, 60× `<img>`) + 字下げ              |
//! | shimizu-kagaku      | 割り注 + 傍点 + ルビ (bar + base forms)                |
//! | terada-tosa         | 傍点 (bouten, 84×) + 外字 + 字下げ + 地付き            |
//! | toyoshima-kamoryo   | near-plain (pure paragraphs)                          |
//! | watanabe-hanayome   | 縦中横 + 太字 + 字下げ + 外字 + ルビ                   |

use aozora::Document;
use aozora_conformance::{RenderFixture, fixtures_root};
use pretty_assertions::assert_eq;

#[test]
fn works_gate_html_matches_golden() {
    let fixtures = load_works_fixtures();
    for fixture in &fixtures {
        let doc = Document::new(fixture.source.clone());
        let actual = doc.parse().to_html();
        let expected = fixture.html_golden(&actual);
        assert_eq!(
            actual, expected,
            "html drift for vendored work {}",
            fixture.name,
        );
    }
}

fn load_works_fixtures() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "works");
    assert!(!fixtures.is_empty(), "no works fixtures found");
    fixtures
}
