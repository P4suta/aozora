//! Walks `AOZORA_CORPUS_ROOT` and verifies every document parses
//! cleanly and round-trips through `parse ∘ serialize`.
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never
//! hard-fails on missing corpus.

use std::str;

use aozora::Document;
use aozora_encoding::decode_sjis;

#[test]
fn corpus_round_trip_is_a_fixed_point() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping corpus sweep");
        return;
    };

    let mut count: usize = 0;
    let mut decode_fallbacks: usize = 0;

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");

        let utf8 = if let Ok(s) = decode_sjis(&item.bytes) {
            s
        } else if let Ok(s) = str::from_utf8(&item.bytes) {
            decode_fallbacks += 1;
            s.to_owned()
        } else {
            eprintln!("skip (neither SJIS nor UTF-8): {}", item.label);
            continue;
        };

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
        "corpus sweep: {count} docs walked ({decode_fallbacks} UTF-8 fallback after SJIS decode failure)"
    );
}
