//! Walks `AOZORA_CORPUS_ROOT` and verifies every document parses
//! cleanly and round-trips through `parse ∘ serialize`.
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never
//! hard-fails on missing corpus.

use std::borrow::Cow;

use aozora::Document;
use aozora_encoding::decode_auto;

#[test]
fn corpus_round_trip_is_a_fixed_point() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping corpus sweep");
        return;
    };

    let mut count: usize = 0;
    let mut sjis_decoded: usize = 0;

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");

        let Ok(utf8) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };
        if matches!(utf8, Cow::Owned(_)) {
            sjis_decoded += 1;
        }

        // Parse must not panic and must produce a tree.
        let doc = Document::new(utf8);
        let tree = doc.parse();
        let serialized = tree.serialize();

        // Round-trip stability: parse ∘ serialize is a fixed point.
        let doc2 = Document::new(serialized.clone());
        let tree2 = doc2.parse();
        let serialized2 = tree2.serialize();

        assert_eq!(
            serialized, serialized2,
            "round-trip is not a fixed point for {}",
            item.label
        );

        count += 1;
    }

    eprintln!(
        "corpus sweep: {count} docs walked ({sjis_decoded} decoded from Shift_JIS, the rest already UTF-8)"
    );
}
