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
//! Coverage: the curated set (`tests/common/mod.rs`, one of every node kind),
//! every `crates/aozora-conformance/fixtures/render/**` source, and — when
//! `AOZORA_CORPUS_ROOT` is set — every corpus document (skipped cleanly when
//! unset, as it is in CI; the curated + conformance inputs carry the gate).

mod common;

use std::borrow::Cow;

use aozora::Document;
use aozora::render::serialize_owned;
use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_encoding::decode_auto;
use common::CURATED;

/// Assert the owned serializer reproduces the borrowed `to_source` byte-exactly.
fn assert_owned_matches(src: &str) {
    let doc = Document::new(src);
    let tree = doc.parse();
    let owned = doc.parse_owned();
    assert_eq!(
        serialize_owned(&owned),
        tree.to_source(),
        "owned serializer diverged from borrowed for {src:?}"
    );
}

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
