//! Differential gate for the #237 owned-AST HTML renderer (P0.2b).
//!
//! Proves the owned renderer reproduces the borrowed HTML **byte-for-byte**:
//! for every input, the owned path `render_html_owned(parse_owned(src))` must
//! equal the borrowed authority `render_to_string(lex(src))`. The two share the
//! same normalized text and position-isomorphic registry, so byte-equality is
//! exactly the proof that every `render_*_owned` reproduces its borrowed twin
//! (paragraph wrapping, `<br />`, container open/close, escaping, gaiji
//! resolution, heading tags) and every `StrId`/range resolve recovers the
//! original `&str`.
//!
//! Since P0.3 (the public `Tree` API flip), `Tree::to_html()` *is* the owned
//! path, so the gate compares directly against the borrowed renderer
//! (`aozora::html::render_to_string`), which remains the byte authority until
//! the borrowed AST is deleted in step 4 (then the gate re-anchors onto the
//! conformance golden + corpus). The render fixtures' golden HTML — produced
//! by the borrowed renderer — also cross-checks both.
//!
//! Coverage: the curated set (`tests/common/mod.rs`, one of every node kind),
//! every `crates/aozora-conformance/fixtures/render/**` source, and — when
//! `AOZORA_CORPUS_ROOT` is set — every corpus document (skipped cleanly when
//! unset, as it is in CI).

mod common;

use std::borrow::Cow;

use aozora::html::render_to_string as render_html_borrowed;
use aozora::render::render_html_owned;
use aozora::{Arena, Document, lex};
use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_encoding::decode_auto;
use common::CURATED;

/// Assert the owned renderer reproduces the borrowed renderer byte-exactly.
fn assert_owned_matches(src: &str) {
    let owned = Document::new(src).parse_owned();
    let arena = Arena::new();
    let borrowed = lex(src, &arena);
    assert_eq!(
        render_html_owned(&owned),
        render_html_borrowed(&borrowed),
        "owned HTML renderer diverged from borrowed for {src:?}"
    );
}

#[test]
fn owned_html_matches_borrowed_on_curated_inputs() {
    for src in CURATED {
        assert_owned_matches(src);
    }
}

#[test]
fn owned_html_matches_borrowed_on_render_fixtures() {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "render fixtures must be present");
    for f in fixtures {
        assert_owned_matches(&f.source);
    }
}

#[test]
fn owned_html_matches_borrowed_on_corpus() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping owned-html corpus gate");
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
